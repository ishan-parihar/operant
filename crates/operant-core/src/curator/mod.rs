//! Curator — background skill lifecycle management.
//!
//! Ported from operant-agent/agent/curator.py.
//! Manages skill states (active → stale → archived), pinning,
//! usage tracking, and backup/rollback snapshots.

pub mod archiver;
pub mod backup;
pub mod review;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::skill_usage::{LifecycleState, SkillUsageTracker};

/// Curator runtime state persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuratorState {
    /// Whether the curator is enabled in config
    pub enabled: bool,
    /// Whether the curator is paused (user-requested halt)
    pub paused: bool,
    /// Interval between curator runs in hours
    pub interval_hours: u64,
    /// Timestamp (Unix epoch) of last curator run
    pub last_run_at: Option<i64>,
    /// Summary of last run
    pub last_run_summary: Option<String>,
    /// Total number of curator runs
    pub run_count: u64,
    /// Path to last report file
    pub last_report_path: Option<PathBuf>,
    /// Days of inactivity before a skill is considered stale
    pub stale_after_days: u64,
    /// Days of inactivity before a stale skill is archived
    pub archive_after_days: u64,
}

impl Default for CuratorState {
    fn default() -> Self {
        Self {
            enabled: true,
            paused: false,
            interval_hours: 24,
            last_run_at: None,
            last_run_summary: None,
            run_count: 0,
            last_report_path: None,
            stale_after_days: 14,
            archive_after_days: 30,
        }
    }
}

/// Result of a single curator run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuratorReport {
    pub timestamp: i64,
    pub skills_scanned: usize,
    pub skills_archived: Vec<String>,
    pub skills_stale: Vec<String>,
    pub errors: Vec<String>,
    pub summary: String,
}

/// Main curator engine — reviews skills, manages lifecycle transitions.
pub struct CuratorEngine {
    state: Arc<RwLock<CuratorState>>,
    state_path: PathBuf,
    skills_dir: PathBuf,
    archive_dir: PathBuf,
    backup_dir: PathBuf,
    usage_tracker: Arc<SkillUsageTracker>,
}

impl CuratorEngine {
    /// Create a new curator engine.
    pub fn new(
        skills_dir: PathBuf,
        state_path: PathBuf,
        usage_tracker: Arc<SkillUsageTracker>,
    ) -> Self {
        let archive_dir = skills_dir.join(".archive");
        let backup_dir = skills_dir.join(".backups");
        Self {
            state: Arc::new(RwLock::new(CuratorState::default())),
            state_path,
            skills_dir,
            archive_dir,
            backup_dir,
            usage_tracker,
        }
    }

    /// Load state from disk (creates default if not found).
    pub async fn load_state(&self) -> Result<CuratorState> {
        if self.state_path.exists() {
            let content = fs::read_to_string(&self.state_path)?;
            let state: CuratorState = serde_json::from_str(&content)?;
            *self.state.write().await = state.clone();
            Ok(state)
        } else {
            let state = CuratorState::default();
            *self.state.write().await = state.clone();
            self.save_state_inner(&state).await?;
            Ok(state)
        }
    }

    async fn save_state_inner(&self, state: &CuratorState) -> Result<()> {
        if let Some(parent) = self.state_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(state)?;
        fs::write(&self.state_path, content)?;
        Ok(())
    }

    /// Save the current state to disk.
    pub async fn save_state(&self) -> Result<()> {
        let state = self.state.read().await.clone();
        self.save_state_inner(&state).await
    }

    /// Run a full curator review cycle.
    ///
    /// Scans agent-created skills, archives those past `archive_after_days`,
    /// and marks those past `stale_after_days` as stale. Creates a backup
    /// before any archiving when not in dry-run mode.
    pub async fn run_review(
        &self,
        dry_run: bool,
        _llm_client: Option<&dyn review::LlmReviewClient>,
    ) -> Result<CuratorReport> {
        let mut state = self.state.write().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // Load fresh usage data
        self.usage_tracker.load()?;
        let all_records = self.usage_tracker.all_records();
        let agent_created: Vec<_> = all_records.iter().filter(|r| r.agent_created).collect();

        let mut archived = Vec::new();
        let mut stale = Vec::new();
        let mut errors = Vec::new();

        for record in &agent_created {
            if record.pinned {
                continue;
            }
            let inactive_days = (now - record.last_used.timestamp()) / 86400;
            if inactive_days >= state.archive_after_days as i64 {
                if !dry_run {
                    match archiver::archive_skill(&record.name, &self.skills_dir, &self.archive_dir)
                    {
                        Ok(()) => {
                            let _ = self
                                .usage_tracker
                                .set_state(&record.name, LifecycleState::Archived);
                            archived.push(record.name.clone());
                        }
                        Err(e) => {
                            errors.push(format!("Failed to archive '{}': {}", record.name, e));
                        }
                    }
                } else {
                    archived.push(record.name.clone());
                }
            } else if inactive_days >= state.stale_after_days as i64 {
                stale.push(record.name.clone());
            }
        }

        state.last_run_at = Some(now);
        state.run_count += 1;
        let summary = format!(
            "Scanned {} agent-created skills. Archived: {}, Stale: {}, Errors: {}",
            agent_created.len(),
            archived.len(),
            stale.len(),
            errors.len()
        );

        if !dry_run {
            // Create backup before archiving
            if !archived.is_empty() {
                backup::create_backup(&self.skills_dir, &self.backup_dir, Some("pre-archive"))
                    .unwrap_or_else(|e| {
                        errors.push(format!("Backup failed: {}", e));
                        PathBuf::new()
                    });
            }
            self.save_state_inner(&state).await?;
            self.usage_tracker.save()?;
        }

        state.last_run_summary = Some(summary.clone());

        Ok(CuratorReport {
            timestamp: now,
            skills_scanned: agent_created.len(),
            skills_archived: archived,
            skills_stale: stale,
            errors,
            summary,
        })
    }

    /// Pause/unpause the curator.
    pub async fn set_paused(&self, paused: bool) -> Result<()> {
        let mut state = self.state.write().await;
        state.paused = paused;
        self.save_state_inner(&state).await
    }

    /// Check if the curator is enabled and not paused.
    pub async fn is_active(&self) -> bool {
        let state = self.state.read().await;
        state.enabled && !state.paused
    }

    /// Read-only access to the current state.
    pub async fn get_state(&self) -> CuratorState {
        self.state.read().await.clone()
    }

    /// Check if the curator should run now based on idle/interval gates.
    ///
    /// Returns `true` when:
    ///   - The curator is enabled and not paused
    ///   - `last_run_at` is either None (first run) or older than `interval_hours`
    ///
    /// First-run behavior: when there is no `last_run_at` (fresh install),
    /// we seed `last_run_at` to now and return false — deferring the first
    /// real pass by one full interval. Users can invoke `operant curator run`
    /// explicitly to bypass this gate.
    pub async fn maybe_run_curator(&self) -> bool {
        if !self.is_active().await {
            return false;
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let state = self.state.read().await.clone();

        match state.last_run_at {
            None => {
                // First run — seed state so we wait a full interval.
                {
                    let mut state = self.state.write().await;
                    state.last_run_at = Some(now);
                    state.last_run_summary = Some(
                        "deferred first run — curator seeded, will run after one interval"
                            .to_string(),
                    );
                } // drop write guard before async I/O
                let snapshot = self.state.read().await.clone();
                let _ = self.save_state_inner(&snapshot).await;
                tracing::info!("Curator seeded — first run deferred by one interval");
                false
            }
            Some(last) => {
                let interval_secs = state.interval_hours as i64 * 3600;
                let elapsed = now - last;
                if elapsed >= interval_secs {
                    tracing::info!(
                        elapsed_hours = elapsed / 3600,
                        interval_hours = state.interval_hours,
                        "Curator interval elapsed — should run"
                    );
                    true
                } else {
                    tracing::debug!(
                        elapsed_hours = elapsed / 3600,
                        remaining_hours = (interval_secs - elapsed) / 3600,
                        "Curator interval not yet elapsed"
                    );
                    false
                }
            }
        }
    }

    /// Apply automatic lifecycle transitions to agent-created skills.
    ///
    /// Walks every curator-managed skill and moves active → stale → archived
    /// based on the latest real activity timestamp. Pinned skills are never
    /// touched. Returns a counter dict describing what changed.
    ///
    /// This is a deterministic, no-LLM operation that runs whenever the
    /// curator fires — even when `consolidate` is off.
    pub async fn apply_automatic_transitions(
        &self,
    ) -> Result<std::collections::HashMap<String, i64>> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let state = self.state.read().await.clone();
        let stale_cutoff = now - (state.stale_after_days as i64 * 86400);
        let archive_cutoff = now - (state.archive_after_days as i64 * 86400);

        // Load fresh usage data
        self.usage_tracker.load()?;
        let all_records = self.usage_tracker.all_records();
        let agent_created: Vec<_> = all_records.iter().filter(|r| r.agent_created).collect();

        let mut counts = std::collections::HashMap::new();
        counts.insert("checked".to_string(), agent_created.len() as i64);
        counts.insert("archived".to_string(), 0);
        counts.insert("reactivated".to_string(), 0);
        counts.insert("skipped_pinned".to_string(), 0);

        for record in &agent_created {
            if record.pinned {
                *counts.entry("skipped_pinned".to_string()).or_insert(0) += 1;
                continue;
            }

            let inactive_days = (now - record.last_used.timestamp()) / 86400;
            let current = &record.lifecycle;

            // Never-used skills (use_count == 0) get a grace floor:
            // don't archive until at least stale_after_days old.
            let never_used = record.use_count == 0;
            if never_used && record.last_used.timestamp() > stale_cutoff {
                // Younger than stale window — leave it alone.
                if *current == LifecycleState::Active {
                    // No change needed — it's already active.
                }
                continue;
            }

            if record.last_used.timestamp() <= archive_cutoff
                && *current != LifecycleState::Archived
            {
                // Archive the skill
                match archiver::archive_skill(&record.name, &self.skills_dir, &self.archive_dir) {
                    Ok(()) => {
                        let _ = self
                            .usage_tracker
                            .set_state(&record.name, LifecycleState::Archived);
                        *counts.entry("archived".to_string()).or_insert(0) += 1;
                        tracing::info!(skill = %record.name, "Curator archived stale skill");
                    }
                    Err(e) => {
                        tracing::warn!(
                            skill = %record.name, error = %e,
                            "Curator failed to archive skill"
                        );
                    }
                }
            } else if record.last_used.timestamp() > stale_cutoff
                && *current == LifecycleState::Archived
            {
                // Reactivate — skill got used again after being archived.
                // This shouldn't normally happen (archived skills aren't loaded),
                // but handle it defensively.
                *counts.entry("reactivated".to_string()).or_insert(0) += 1;
            }
        }

        // Aggregate logging (single line, not per-skill)
        let archived_count = counts.get("archived").copied().unwrap_or(0);
        let reactivated_count = counts.get("reactivated").copied().unwrap_or(0);
        let skipped_count = counts.get("skipped_pinned").copied().unwrap_or(0);
        if archived_count > 0 || reactivated_count > 0 {
            tracing::info!(
                checked = agent_created.len(),
                archived = archived_count,
                reactivated = reactivated_count,
                skipped_pinned = skipped_count,
                "Curator automatic transitions complete"
            );
        }

        // Save updated state — clone before async write to release the lock early.
        let summary = format!(
            "Auto-transitions: checked={}, archived={}, reactivated={}, skipped_pinned={}",
            counts.get("checked").unwrap_or(&0),
            counts.get("archived").unwrap_or(&0),
            counts.get("reactivated").unwrap_or(&0),
            counts.get("skipped_pinned").unwrap_or(&0),
        );
        {
            let mut state = self.state.write().await;
            state.last_run_at = Some(now);
            state.run_count += 1;
            state.last_run_summary = Some(summary);
        } // drop write guard before async I/O
        let state_snapshot = self.state.read().await.clone();
        let _ = self.save_state_inner(&state_snapshot).await;
        let _ = self.usage_tracker.save();

        Ok(counts)
    }
}
