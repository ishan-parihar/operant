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
}
