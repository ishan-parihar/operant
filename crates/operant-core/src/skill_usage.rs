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
    /// Total patch/edit count (plan 007: the legacy `.usage.json`
    /// `patch_count` field is preserved here so the existing
    /// concurrent-write contract is observable).
    #[serde(default)]
    pub patch_count: u64,
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
    /// Returns an empty store if the file doesn't exist, and falls back to an
    /// empty store (with a warning) when the file is present but corrupt —
    /// telemetry is disposable and must never brick the curator or the skill
    /// manager. IO errors (permissions, etc.) still propagate. (R21)
    pub fn load(filepath: &Path) -> Result<Self> {
        if !filepath.exists() {
            return Ok(Self::new());
        }
        let content = std::fs::read_to_string(filepath)?;
        match serde_json::from_str::<Vec<UsageRecord>>(&content) {
            Ok(records) => Ok(Self { records }),
            Err(e) => {
                tracing::warn!(
                    file = %filepath.display(),
                    error = %e,
                    "corrupt skill usage telemetry — starting fresh"
                );
                Ok(Self::new())
            }
        }
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

    #[expect(
        clippy::expect_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    /// Ensure a record exists, creating a default one if needed.
    fn ensure_record(&mut self, name: &str) -> &mut UsageRecord {
        if !self.records.iter().any(|r| r.name == name) {
            self.records.push(UsageRecord {
                name: name.to_string(),
                version: String::new(),
                first_used: Utc::now(),
                last_used: Utc::now(),
                use_count: 0,
                patch_count: 0,
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
        rec.patch_count += 1;
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

    /// Record a skill creation event (hermes `skill_manager_tool.py`
    /// `record_created` parity). `agent_created` is true only when the
    /// background-review fork created the skill — those are the records the
    /// curator manages (archive/stale review). Ordinary creates stay tracked
    /// but are never auto-archived.
    ///
    /// Note: non-review creates keep `provenance` unset (only
    /// `mark_agent_created` sets `"agent"`); nothing queries provenance for
    /// non-agent-created records, so this is purely cosmetic. (R21)
    pub fn record_created(&mut self, name: &str, agent_created: bool) {
        if agent_created {
            self.mark_agent_created(name);
        } else {
            self.ensure_record(name);
        }
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

/// Take an OS-level exclusive advisory lock on `<path>.lock` for the duration
/// of `f`, making read-modify-write cycles on `path` atomic across processes
/// (hermes's `.json.lock` parity). The kernel releases the lock when the file
/// handle drops or the process exits, so a crash can never leave a stale lock
/// that wedges later writers.
///
/// The `.lock` files intentionally persist: unlinking a lockfile is racy (a
/// new opener would create a fresh inode and no longer contend with existing
/// fd holders), so they are left in place as inert dotfiles that no skill
/// scan reads.
///
/// Best-effort: if the lock file cannot be opened we proceed unlocked rather
/// than fail the operation — telemetry must never block skill tooling.
pub(crate) fn with_exclusive_file_lock<T>(path: &Path, f: impl FnOnce() -> T) -> T {
    let lock_path = PathBuf::from(format!("{}.lock", path.display()));
    if let Some(parent) = lock_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&lock_path)
    {
        Ok(file) => {
            let _ = file.lock();
            let result = f();
            let _ = file.unlock();
            result
        }
        Err(_) => f(),
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

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
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

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Save telemetry to disk.
    pub fn save(&self) -> Result<()> {
        let inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.save(&self.file_path)
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Get all records.
    pub fn all_records(&self) -> Vec<UsageRecord> {
        let inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.all_records().to_vec()
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Get agent-created records only.
    pub fn agent_created_records(&self) -> Vec<UsageRecord> {
        let inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.agent_created_records().into_iter().cloned().collect()
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// List active (non-archived/non-retired) records.
    pub fn list_active(&self) -> Vec<UsageRecord> {
        let inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.list_active().into_iter().cloned().collect()
    }

    /// Read the pinned flag for a skill. Plan 007: replaces the
    /// legacy `.usage.json` reader in `skills_tool::is_pinned`. A
    /// missing record returns `false` (the unpinned default) to
    /// match the previous semantics.
    pub fn is_pinned(&self, name: &str) -> bool {
        let inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner
            .records
            .iter()
            .find(|r| r.name == name)
            .map(|r| r.pinned)
            .unwrap_or(false)
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Set pinned status for a skill.
    pub fn set_pinned(&self, name: &str, pinned: bool) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.set_pinned(name, pinned)
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Mark a skill as agent-created.
    pub fn mark_agent_created(&self, name: &str) {
        let mut inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.mark_agent_created(name);
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Record a skill creation event. See [`UsageTelemetry::record_created`].
    pub fn record_created(&self, name: &str, agent_created: bool) {
        let mut inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.record_created(name, agent_created);
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Bump `last_used` for a patch/edit event. See [`UsageTelemetry::bump_patch`].
    pub fn bump_patch(&self, name: &str) {
        let mut inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.bump_patch(name);
    }

    /// Add `count` to a record's `patch_count` (creating the record
    /// if it doesn't exist), and bump `last_used`. The legacy
    /// `.usage.json` migration uses this to merge per-skill counts
    /// into the curator store. (Plan 007)
    pub fn add_patch_count(&self, name: &str, count: u64) {
        let mut inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        let rec = inner.ensure_record(name);
        rec.patch_count = rec.patch_count.saturating_add(count);
        rec.last_used = Utc::now();
    }

    /// Plan 007: one-shot migration of the legacy `.usage.json` sidecar.
    ///
    /// Reads the legacy file at `legacy_path`, adds each entry's
    /// `patch_count` to the corresponding record's `patch_count` in
    /// the curator tracker, then deletes the legacy file. The
    /// merge is additive — a record that already has
    /// `patch_count = 5` and a legacy file reporting `3` ends at 8.
    ///
    /// Returns `Ok(true)` if the legacy file was found and migrated,
    /// `Ok(false)` if no legacy file existed. A corrupt legacy
    /// file is logged and removed; the curator store is left
    /// untouched.
    pub fn migrate_from_legacy(&self, legacy_path: &Path) -> std::io::Result<bool> {
        if !legacy_path.exists() {
            return Ok(false);
        }
        let raw = match std::fs::read_to_string(legacy_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    legacy = %legacy_path.display(),
                    error = %e,
                    "migrate_from_legacy: unreadable legacy file, removing"
                );
                let _ = std::fs::remove_file(legacy_path);
                return Ok(true);
            }
        };
        let parsed: std::collections::HashMap<String, serde_json::Value> =
            match serde_json::from_str(&raw) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(
                        legacy = %legacy_path.display(),
                        error = %e,
                        "migrate_from_legacy: invalid JSON in legacy file, removing"
                    );
                    let _ = std::fs::remove_file(legacy_path);
                    return Ok(true);
                }
            };
        // `with_exclusive_lock` re-loads fresh state from disk under the
        // OS file lock, then auto-saves the inner telemetry on drop.
        let _ = self.with_exclusive_lock(|t| {
            for (name, entry) in parsed {
                let patch_count = entry
                    .get("patch_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                if patch_count == 0 {
                    continue;
                }
                t.add_patch_count(&name, patch_count);
            }
            Ok(())
        });
        if let Err(e) = std::fs::remove_file(legacy_path) {
            tracing::warn!(
                legacy = %legacy_path.display(),
                error = %e,
                "migrate_from_legacy: curator store updated but legacy file removal failed"
            );
        } else {
            tracing::info!(
                legacy = %legacy_path.display(),
                "migrate_from_legacy: legacy .usage.json merged into curator tracker and removed"
            );
        }
        Ok(true)
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Remove a record by name.
    pub fn remove(&self, name: &str) {
        let mut inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.remove(name);
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Set the lifecycle state for a skill.
    pub fn set_state(&self, name: &str, state: LifecycleState) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .expect("skill usage mutex poisoned — programmer error");
        inner.set_state(name, state)
    }

    /// Run `f` as a single read-modify-write transaction on the backing file:
    /// re-load fresh state from disk under a cross-process exclusive file lock
    /// (so we never clobber another process's newer writes with stale in-memory
    /// state), apply `f`, then persist. This makes the agent's skill_manage
    /// bridge and `operant curator` pin/unpin/restore/archive atomic with
    /// respect to each other. (R21)
    pub fn with_exclusive_lock<R>(&self, f: impl FnOnce(&Self) -> Result<R>) -> Result<R> {
        with_exclusive_file_lock(&self.file_path, || {
            self.load()?;
            let result = f(self)?;
            self.save()?;
            Ok(result)
        })
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
    fn test_load_tolerates_corrupt_file() {
        let path = test_file_path();
        std::fs::write(&path, "{ definitely not json").unwrap();
        let telemetry = UsageTelemetry::load(&path).unwrap();
        assert!(telemetry.is_empty());
        // Cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_record_created_honors_agent_created_flag() {
        let mut telemetry = UsageTelemetry::new();
        telemetry.record_created("review-skill", true);
        telemetry.record_created("user-skill", false);

        let review = telemetry.get_record("review-skill").unwrap();
        assert!(review.agent_created);
        assert_eq!(review.provenance.as_deref(), Some("agent"));

        let user = telemetry.get_record("user-skill").unwrap();
        assert!(!user.agent_created);
    }

    #[test]
    fn test_tracker_bump_patch_and_remove() {
        let path = test_file_path();
        let tracker = SkillUsageTracker::new(path.clone());
        tracker.load().unwrap();
        tracker.record_created("s1", true);
        tracker.bump_patch("s1");
        tracker.save().unwrap();

        let loaded = SkillUsageTracker::new(path.clone());
        loaded.load().unwrap();
        let rec = loaded
            .all_records()
            .into_iter()
            .find(|r| r.name == "s1")
            .expect("record survives save/load");
        assert!(rec.agent_created);
        assert!(rec.last_used.timestamp() > 0);

        loaded.remove("s1");
        loaded.save().unwrap();
        let reloaded = SkillUsageTracker::new(path.clone());
        reloaded.load().unwrap();
        assert!(reloaded.all_records().is_empty());
        // Cleanup
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}.lock", path.display()));
    }

    #[test]
    fn tracker_with_exclusive_lock_serializes_writers() {
        // flock contends per open-file-description, so separate tracker
        // instances in separate threads exercise the same cross-process
        // serialization `operant curator` relies on: no record is lost.
        let path = test_file_path();
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let tracker = SkillUsageTracker::new(path);
                    for j in 0..25 {
                        tracker
                            .with_exclusive_lock(|t| {
                                t.bump_patch(&format!("skill-{}", (i + j) % 4));
                                Ok(())
                            })
                            .expect("locked transaction succeeds");
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let tracker = SkillUsageTracker::new(path.clone());
        tracker.load().unwrap();
        let records = tracker.all_records();
        assert_eq!(
            records.len(),
            4,
            "all four skills survive concurrent writers"
        );
        let mut names: Vec<_> = records.iter().map(|r| r.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["skill-0", "skill-1", "skill-2", "skill-3"]);
        // Cleanup
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}.lock", path.display()));
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

    /// Plan 007 / test plan: seed a legacy `.usage.json` and prove
    /// the migrator merges `patch_count` additively into the curator
    /// tracker, then removes the legacy file.
    #[test]
    fn migrate_from_legacy_merges_patch_counts_then_removes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let curator_dir = root.join(".curator");
        std::fs::create_dir_all(&curator_dir).expect("mkdir .curator");
        let curator_path = curator_dir.join("usage.json");
        let legacy_path = root.join(".usage.json");

        // Seed a curator record with a non-zero patch_count so we can
        // prove additive merge (not overwrite).
        let tracker = SkillUsageTracker::new(curator_path.clone());
        tracker.with_exclusive_lock(|t| {
            t.bump_patch("skill-a");
            t.bump_patch("skill-a");
            Ok(())
        }).expect("seed");
        // After 2 bumps, skill-a.patch_count = 2.

        // Seed the legacy file: skill-a already at 2, legacy adds 3
        // (should become 5); skill-b is new and gets 4.
        let legacy = serde_json::json!({
            "skill-a": { "use_count": 0, "patch_count": 3 },
            "skill-b": { "use_count": 0, "patch_count": 4 }
        });
        std::fs::write(&legacy_path, serde_json::to_string_pretty(&legacy).unwrap())
            .expect("seed legacy");

        // Run migration.
        let did = tracker
            .migrate_from_legacy(&legacy_path)
            .expect("migrate ok");
        assert!(did, "migrate must report it did something");
        assert!(!legacy_path.exists(), "legacy file must be removed");

        // Curator store carries merged counts.
        let records: Vec<UsageRecord> = serde_json::from_str(
            &std::fs::read_to_string(&curator_path).expect("curator readable"),
        )
        .expect("valid JSON");
        let a = records.iter().find(|r| r.name == "skill-a").expect("a");
        let b = records.iter().find(|r| r.name == "skill-b").expect("b");
        assert_eq!(a.patch_count, 5, "skill-a: 2 (existing) + 3 (legacy) = 5");
        assert_eq!(b.patch_count, 4, "skill-b: 0 + 4 = 4");
    }

    #[test]
    fn migrate_from_legacy_returns_false_when_no_legacy_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let curator_dir = root.join(".curator");
        std::fs::create_dir_all(&curator_dir).expect("mkdir");
        let tracker = SkillUsageTracker::new(curator_dir.join("usage.json"));
        let did = tracker
            .migrate_from_legacy(&root.join(".usage.json"))
            .expect("migrate ok");
        assert!(!did, "no legacy file => migrate is a no-op");
    }

    #[test]
    fn migrate_from_legacy_tolerates_corrupt_legacy_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let curator_dir = root.join(".curator");
        std::fs::create_dir_all(&curator_dir).expect("mkdir");
        let tracker = SkillUsageTracker::new(curator_dir.join("usage.json"));
        let legacy_path = root.join(".usage.json");
        std::fs::write(&legacy_path, b"this is not json").expect("seed corrupt");
        let did = tracker
            .migrate_from_legacy(&legacy_path)
            .expect("migrate ok (corrupt input is removed, not an Err)");
        assert!(did, "corrupt legacy must be cleaned up");
        assert!(!legacy_path.exists(), "corrupt file must be removed");
    }
}
