//! Skill usage telemetry — atomic usage tracking sidecar.
//!
//! Tracks per-skill usage metadata in a companion JSON file.
//! Ported from `operant-agent/tools/skill_usage.py`.
//!
//! Design notes:
//! - Sidecar, not frontmatter. Keeps operational telemetry out of user-authored
//!   SKILL.md content and avoids conflict pressure for bundled/hub skills.
//! - Atomic writes via temp file + rename (same pattern as bundled manifest).
//! - Provenance: agent_created flag distinguishes agent-vs-user authored skills.
//!
//! Lifecycle states:
//! - Active: default state
//! - Deprecated: skill is deprecated but still present
//! - Archived: moved to archive; not active but restorable
//! - Retired: permanently retired

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Lifecycle state for a skill.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum LifecycleState {
    /// Default state — skill is active and usable.
    #[default]
    Active,
    /// Skill is deprecated but still present.
    Deprecated,
    /// Skill has been archived (moved to .archive/).
    Archived,
    /// Skill is permanently retired.
    Retired,
}

/// Per-skill usage telemetry record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    /// Skill name (matches SKILL.md frontmatter `name:` field).
    pub name: String,
    /// Skill version at time of recording.
    pub version: String,
    /// First time this skill was used.
    pub first_used: DateTime<Utc>,
    /// Most recent time this skill was used, viewed, or patched.
    pub last_used: DateTime<Utc>,
    /// Total usage count.
    pub use_count: u64,
    /// Whether this skill was explicitly created by the agent.
    #[serde(default)]
    pub agent_created: bool,
    /// Current lifecycle state.
    #[serde(default)]
    pub lifecycle: LifecycleState,
    /// Provenance source (e.g., "agent", "hub", "bundled").
    #[serde(default)]
    pub provenance: Option<String>,
    /// Whether this skill is pinned (exempt from auto-archive).
    #[serde(default)]
    pub pinned: bool,
}

/// Wrapper around a collection of usage records with load/save from JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageTelemetry {
    /// Inner list of usage records.
    records: Vec<UsageRecord>,
}

impl UsageTelemetry {
    /// Create a new empty telemetry store.
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    /// Load telemetry from a JSON file.
    ///
    /// Returns an empty store if the file doesn't exist.
    /// Propagates IO / JSON parse errors when the file is present but corrupt.
    pub fn load(filepath: &Path) -> Result<Self> {
        if !filepath.exists() {
            return Ok(Self::new());
        }
        let content = std::fs::read_to_string(filepath)?;
        let records: Vec<UsageRecord> = serde_json::from_str(&content)?;
        Ok(Self { records })
    }

    /// Save telemetry to a JSON file atomically (temp file + rename).
    pub fn save(&self, filepath: &Path) -> Result<()> {
        if let Some(parent) = filepath.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Use a temp file for atomic writes
        let tmp_path = filepath.with_extension("json.tmp");
        let content = serde_json::to_string_pretty(&self.records)?;
        std::fs::write(&tmp_path, &content)?;
        std::fs::rename(&tmp_path, filepath)?;
        Ok(())
    }

    /// Get a record by skill name.
    pub fn get_record(&self, name: &str) -> Option<&UsageRecord> {
        self.records.iter().find(|r| r.name == name)
    }

    /// Get a mutable record by skill name.
    pub fn get_record_mut(&mut self, name: &str) -> Option<&mut UsageRecord> {
        self.records.iter_mut().find(|r| r.name == name)
    }

    /// Ensure a record exists, creating a default one if needed.
    fn ensure_record(&mut self, name: &str) -> &mut UsageRecord {
        if !self.records.iter().any(|r| r.name == name) {
            self.records.push(UsageRecord {
                name: name.to_string(),
                version: String::new(),
                first_used: Utc::now(),
                last_used: Utc::now(),
                use_count: 0,
                agent_created: false,
                lifecycle: LifecycleState::Active,
                provenance: None,
                pinned: false,
            });
        }
        self.get_record_mut(name).expect("record was just inserted")
    }

    /// Bump usage count and update `last_used`.
    ///
    /// Called when a skill is actively used (e.g., loaded into the prompt path).
    pub fn bump_use(&mut self, name: &str) {
        let rec = self.ensure_record(name);
        rec.use_count += 1;
        rec.last_used = Utc::now();
    }

    /// Update `last_used` only (for view events).
    ///
    /// Called from skill_view or skill discovery.
    pub fn bump_view(&mut self, name: &str) {
        let rec = self.ensure_record(name);
        rec.last_used = Utc::now();
    }

    /// Update `last_used` only (for patch install events).
    ///
    /// Called when a skill is patched or edited.
    pub fn bump_patch(&mut self, name: &str) {
        let rec = self.ensure_record(name);
        rec.last_used = Utc::now();
    }

    /// Set the lifecycle state for a skill.
    ///
    /// Returns an error if the skill has no record.
    pub fn set_state(&mut self, name: &str, state: LifecycleState) -> Result<()> {
        let rec = self
            .get_record_mut(name)
            .ok_or_else(|| anyhow::anyhow!("Skill '{}' not found in telemetry", name))?;
        rec.lifecycle = state;
        Ok(())
    }

    /// Set lifecycle to Archived.
    pub fn archive(&mut self, name: &str) -> Result<()> {
        self.set_state(name, LifecycleState::Archived)
    }

    /// Restore a skill from Archived back to Active.
    pub fn restore(&mut self, name: &str) -> Result<()> {
        self.set_state(name, LifecycleState::Active)
    }

    /// Set the pinned status for a skill.
    ///
    /// Returns an error if the skill has no record.
    pub fn set_pinned(&mut self, name: &str, pinned: bool) -> Result<()> {
        let rec = self
            .get_record_mut(name)
            .ok_or_else(|| anyhow::anyhow!("Skill '{}' not found in telemetry", name))?;
        rec.pinned = pinned;
        Ok(())
    }

    /// Return records filtered to agent-created skills only.
    pub fn agent_created_records(&self) -> Vec<&UsageRecord> {
        self.records.iter().filter(|r| r.agent_created).collect()
    }

    /// Return active (non-archived, non-retired) records.
    pub fn list_active(&self) -> Vec<&UsageRecord> {
        self.records
            .iter()
            .filter(|r| {
                r.lifecycle != LifecycleState::Archived && r.lifecycle != LifecycleState::Retired
            })
            .collect()
    }

    /// Mark a skill as agent-created and set provenance to "agent".
    pub fn mark_agent_created(&mut self, name: &str) {
        let rec = self.ensure_record(name);
        rec.agent_created = true;
        rec.provenance = Some("agent".to_string());
    }

    /// Get records filtered by provenance source.
    pub fn get_by_provenance(&self, source: &str) -> Vec<&UsageRecord> {
        self.records
            .iter()
            .filter(|r| r.provenance.as_deref() == Some(source))
            .collect()
    }

    /// Remove a record by name.
    pub fn remove(&mut self, name: &str) {
        self.records.retain(|r| r.name != name);
    }

    /// Return all records.
    pub fn all_records(&self) -> &[UsageRecord] {
        &self.records
    }

    /// Return all records (mutable).
    pub fn all_records_mut(&mut self) -> &mut Vec<UsageRecord> {
        &mut self.records
    }

    /// Return the number of records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Returns true if there are no records.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

impl Default for UsageTelemetry {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe wrapper around `UsageTelemetry` for shared access across components.
///
/// Provides atomic load/save from a JSON file, suitable for use behind `Arc`.
pub struct SkillUsageTracker {
    file_path: PathBuf,
    inner: std::sync::Mutex<UsageTelemetry>,
}

impl SkillUsageTracker {
    /// Create a new tracker pointing at the given file path.
    pub fn new(file_path: PathBuf) -> Self {
        Self {
            file_path,
            inner: std::sync::Mutex::new(UsageTelemetry::new()),
        }
    }

    /// Load telemetry from disk (creates empty if missing).
    pub fn load(&self) -> Result<()> {
        let telemetry = UsageTelemetry::load(&self.file_path)?;
        let mut inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        *inner = telemetry;
        Ok(())
    }

    /// Save telemetry to disk.
    pub fn save(&self) -> Result<()> {
        let inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.save(&self.file_path)
    }

    /// Get all records.
    pub fn all_records(&self) -> Vec<UsageRecord> {
        let inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.all_records().to_vec()
    }

    /// Get agent-created records only.
    pub fn agent_created_records(&self) -> Vec<UsageRecord> {
        let inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.agent_created_records().into_iter().cloned().collect()
    }

    /// List active (non-archived/non-retired) records.
    pub fn list_active(&self) -> Vec<UsageRecord> {
        let inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.list_active().into_iter().cloned().collect()
    }

    /// Set pinned status for a skill.
    pub fn set_pinned(&self, name: &str, pinned: bool) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.set_pinned(name, pinned)
    }

    /// Mark a skill as agent-created.
    pub fn mark_agent_created(&self, name: &str) {
        let mut inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.mark_agent_created(name);
    }

    /// Remove a record by name.
    pub fn remove(&self, name: &str) {
        let mut inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.remove(name);
    }

    /// Set the lifecycle state for a skill.
    pub fn set_state(&self, name: &str, state: LifecycleState) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.set_state(name, state)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_file_path() -> std::path::PathBuf {
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "operant_skill_usage_test_{}_{}.json",
            std::process::id(),
            count
        ))
    }

    #[test]
    fn test_new_is_empty() {
        let telemetry = UsageTelemetry::new();
        assert!(telemetry.is_empty());
        assert_eq!(telemetry.len(), 0);
    }

    #[test]
    fn test_load_nonexistent_file() {
        let path = test_file_path();
        let telemetry = UsageTelemetry::load(&path).unwrap();
        assert!(telemetry.is_empty());
        // Cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let path = test_file_path();
        let mut telemetry = UsageTelemetry::new();
        telemetry.bump_use("test-skill");
        telemetry.bump_view("test-skill");
        telemetry.save(&path).unwrap();

        let loaded = UsageTelemetry::load(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        let rec = loaded.get_record("test-skill").unwrap();
        assert_eq!(rec.use_count, 1);
        assert!(!rec.agent_created);
        assert_eq!(rec.lifecycle, LifecycleState::Active);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_bump_use_increments_count() {
        let mut telemetry = UsageTelemetry::new();
        telemetry.bump_use("skill-a");
        telemetry.bump_use("skill-a");
        telemetry.bump_use("skill-a");
        assert_eq!(telemetry.get_record("skill-a").unwrap().use_count, 3);
    }

    #[test]
    fn test_bump_view_does_not_increment_count() {
        let mut telemetry = UsageTelemetry::new();
        telemetry.bump_view("skill-b");
        let rec = telemetry.get_record("skill-b").unwrap();
        assert_eq!(rec.use_count, 0);
        let elapsed = Utc::now() - rec.last_used;
        assert!(elapsed.num_seconds() < 5);
    }

    #[test]
    fn test_bump_patch_does_not_increment_count() {
        let mut telemetry = UsageTelemetry::new();
        telemetry.bump_patch("skill-c");
        let rec = telemetry.get_record("skill-c").unwrap();
        assert_eq!(rec.use_count, 0);
        let elapsed = Utc::now() - rec.last_used;
        assert!(elapsed.num_seconds() < 5);
    }

    #[test]
    fn test_set_state() {
        let mut telemetry = UsageTelemetry::new();
        telemetry.bump_use("test");
        telemetry
            .set_state("test", LifecycleState::Archived)
            .unwrap();
        assert_eq!(
            telemetry.get_record("test").unwrap().lifecycle,
            LifecycleState::Archived
        );
    }

    #[test]
    fn test_set_state_nonexistent_skill_returns_error() {
        let mut telemetry = UsageTelemetry::new();
        let result = telemetry.set_state("nonexistent", LifecycleState::Archived);
        assert!(result.is_err());
    }

    #[test]
    fn test_archive_and_restore() {
        let mut telemetry = UsageTelemetry::new();
        telemetry.bump_use("test");
        telemetry.archive("test").unwrap();
        assert_eq!(
            telemetry.get_record("test").unwrap().lifecycle,
            LifecycleState::Archived
        );
        telemetry.restore("test").unwrap();
        assert_eq!(
            telemetry.get_record("test").unwrap().lifecycle,
            LifecycleState::Active
        );
    }

    #[test]
    fn test_mark_agent_created() {
        let mut telemetry = UsageTelemetry::new();
        telemetry.bump_use("test");
        telemetry.mark_agent_created("test");
        let rec = telemetry.get_record("test").unwrap();
        assert!(rec.agent_created);
        assert_eq!(rec.provenance.as_deref(), Some("agent"));
    }

    #[test]
    fn test_get_by_provenance() {
        let mut telemetry = UsageTelemetry::new();
        telemetry.bump_use("skill-agent");
        telemetry.mark_agent_created("skill-agent");
        telemetry.bump_use("skill-hub");
        // Manually set provenance for skill-hub (not done by bump_use alone)
        if let Some(rec) = telemetry.get_record_mut("skill-hub") {
            rec.provenance = Some("hub".to_string());
        }
        telemetry.bump_use("skill-other");

        let agent_skills = telemetry.get_by_provenance("agent");
        assert_eq!(agent_skills.len(), 1);
        assert_eq!(agent_skills[0].name, "skill-agent");

        let hub_skills = telemetry.get_by_provenance("hub");
        assert_eq!(hub_skills.len(), 1);
        assert_eq!(hub_skills[0].name, "skill-hub");
    }

    #[test]
    fn test_get_record_mut_allows_mutation() {
        let mut telemetry = UsageTelemetry::new();
        telemetry.bump_use("test");
        {
            let rec = telemetry.get_record_mut("test").unwrap();
            rec.use_count = 42;
            rec.version = "2.0.0".to_string();
        }
        let rec = telemetry.get_record("test").unwrap();
        assert_eq!(rec.use_count, 42);
        assert_eq!(rec.version, "2.0.0");
    }

    #[test]
    fn test_get_record_nonexistent_returns_none() {
        let telemetry = UsageTelemetry::new();
        assert!(telemetry.get_record("nonexistent").is_none());
    }

    #[test]
    fn test_remove_record() {
        let mut telemetry = UsageTelemetry::new();
        telemetry.bump_use("test");
        assert!(telemetry.get_record("test").is_some());
        telemetry.remove("test");
        assert!(telemetry.get_record("test").is_none());
    }

    #[test]
    fn test_lifecycle_serialization() {
        // Test serde round-trip for LifecycleState
        let state = LifecycleState::Active;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"active\"");

        let deserialized: LifecycleState = serde_json::from_str("\"archived\"").unwrap();
        assert_eq!(deserialized, LifecycleState::Archived);

        let deprecated: LifecycleState = serde_json::from_str("\"deprecated\"").unwrap();
        assert_eq!(deprecated, LifecycleState::Deprecated);

        let retired: LifecycleState = serde_json::from_str("\"retired\"").unwrap();
        assert_eq!(retired, LifecycleState::Retired);
    }

    #[test]
    fn test_multiple_records_independent() {
        let mut telemetry = UsageTelemetry::new();
        telemetry.bump_use("a");
        telemetry.bump_use("b");
        telemetry.bump_view("a");
        telemetry.bump_patch("c");
        assert_eq!(telemetry.len(), 3);
        assert_eq!(telemetry.get_record("a").unwrap().use_count, 1);
        assert_eq!(telemetry.get_record("b").unwrap().use_count, 1);
        assert_eq!(telemetry.get_record("c").unwrap().use_count, 0);
    }

    #[test]
    fn test_save_creates_parent_directory() {
        let dir = std::env::temp_dir().join(format!("operant_usage_subdir_{}", std::process::id()));
        let path = dir.join("subdir").join(".usage.json");
        let mut telemetry = UsageTelemetry::new();
        telemetry.bump_use("test");
        telemetry.save(&path).unwrap();
        assert!(path.exists(), "File should exist after save");

        // Verify we can load it back
        let loaded = UsageTelemetry::load(&path).unwrap();
        assert_eq!(loaded.len(), 1);

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_bump_use_auto_creates_record() {
        let mut telemetry = UsageTelemetry::new();
        telemetry.bump_use("auto-created");
        let rec = telemetry.get_record("auto-created");
        assert!(rec.is_some(), "Record should be auto-created");
        assert_eq!(rec.unwrap().use_count, 1);
    }

    #[test]
    fn test_default_state_is_active() {
        let mut telemetry = UsageTelemetry::new();
        telemetry.bump_use("test");
        assert_eq!(
            telemetry.get_record("test").unwrap().lifecycle,
            LifecycleState::Active
        );
    }

    #[test]
    fn test_usage_record_serde_roundtrip() {
        let mut telemetry = UsageTelemetry::new();
        telemetry.bump_use("serde-test");
        telemetry.mark_agent_created("serde-test");
        telemetry
            .set_state("serde-test", LifecycleState::Deprecated)
            .unwrap();

        let json = serde_json::to_string_pretty(&telemetry).unwrap();
        let deserialized: UsageTelemetry = serde_json::from_str(&json).unwrap();

        let rec = deserialized.get_record("serde-test").unwrap();
        assert!(rec.agent_created);
        assert_eq!(rec.lifecycle, LifecycleState::Deprecated);
        assert_eq!(rec.use_count, 1);
    }
}
