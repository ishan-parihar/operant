# CLI Parity Upgrade: Hermes-RS Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate all Python-delegated stubs in Hermes-RS and achieve 1:1 functional parity with the Python hermes-agent CLI.

**Architecture:** Three phases — (1) Independence: port standalone stubs (curator, plugins install, claw migration) to native Rust; (2) Infrastructure: port runtime services (gateway engine, ACP server, dashboard, MCP serve); (3) Depth: add missing features (kanban multi-board, command registry, RL CLI). Each phase is independently testable. Phases 1-3 are sequential; tasks within a phase parallelize where indicated.

**Tech Stack:** Rust, tokio, clap, rusqlite, reqwest, axum (new), ratatui (existing), rmcp (existing)

**Base paths:**
- `HERMES_RS` = `/home/ishanp/Documents/GitHub/MY-PROJECTS/HERMES/hermes-rs`
- `HERMES_AGENT` = `/home/ishanp/Documents/GitHub/MY-PROJECTS/HERMES/hermes-agent`

---

## File Structure Map

```
crates/hermes-core/src/
├── curator/                    # NEW: Curator module (Task 1.1)
│   ├── mod.rs                  #   Public API, CuratorState, CuratorEngine
│   ├── backup.rs               #   Backup/rollback logic (tar.gz snapshots)
│   ├── review.rs               #   LLM-driven review logic
│   └── archiver.rs             #   Archive/prune/restore operations
│
├── skill_usage.rs              # EXISTING (extend: add pin/unpin, agent_created filter)

crates/hermes-cli/src/
├── cmd_curator.rs              # MODIFY: Replace stubs with real calls (Task 1.2)
├── cmd_plugins.rs              # MODIFY: Replace install stub (Task 1.3)
├── plugins_install.rs          # NEW: Plugin installation logic (git clone + manifest)
├── cmd_claw.rs                 # MODIFY: Real migrate/cleanup (Task 1.4)
├── claw_migrate.rs             # NEW: OpenClaw→Hermes migration engine
│
├── gateway_runner.rs           # MODIFY: Fix config passthrough, add polling (Task 2.1)
├── cmd_acp.rs                  # MODIFY: Replace stub (Task 2.2)
├── acp_server.rs               # NEW: ACP protocol server
├── cmd_dashboard.rs            # MODIFY: Replace stub (Task 2.3)
├── dashboard_server.rs         # NEW: Dashboard HTTP server (axum)
├── cmd_mcp.rs                  # MODIFY: Replace mcp serve stub (Task 2.4)
├── mcp_serve.rs                # NEW: Hermes-as-MCP-server bridge
│
├── cmd_kanban.rs               # MODIFY: Add boards subcommand (Task 3.1)
├── cmd_rl.rs                   # NEW: RL training subcommand (Task 3.3)
├── commands.rs                 # NEW: Slash command registry (Task 3.2)
│
├── main.rs                     # MODIFY: Wire new commands, wire command registry

crates/hermes-core/src/
├── kanban/
│   ├── mod.rs                  # MODIFY: Add board management
│   ├── boards.rs               # NEW: Board registry (filesystem-backed)
│   └── db.rs                   # MODIFY: Accept board-scoped paths
```

---

## Phase 1: Independence (Eliminate Python Dependencies)

### Task 1.1: Curator Core Engine

**Files:**
- Create: `crates/hermes-core/src/curator/mod.rs`
- Create: `crates/hermes-core/src/curator/backup.rs`
- Create: `crates/hermes-core/src/curator/archiver.rs`
- Create: `crates/hermes-core/src/curator/review.rs`
- Modify: `crates/hermes-core/src/lib.rs` (add `pub mod curator`)
- Test: `crates/hermes-core/tests/test_curator.rs`

**Dependencies:** None (can run in parallel with Tasks 1.3, 1.4)

**Python reference:** `hermes_agent/agent/curator.py`, `hermes_agent/agent/curator_backup.py`, `hermes_agent/tools/skill_usage.py`, `hermes_agent/hermes_cli/curator.py:1-100` (status display logic)

- [ ] **Step 1: Create `curator/mod.rs` with state types**

```rust
//! Curator — background skill lifecycle management.
//!
//! Ported from hermes-agent/agent/curator.py.
//! Manages skill states (active → stale → archived), pinning,
//! usage tracking, and backup/rollback snapshots.

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

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
```

- [ ] **Step 2: Create `curator/backup.rs` with snapshot logic**

```rust
//! Backup and rollback for curator state.
//! Ported from hermes-agent/agent/curator_backup.py.

use std::path::{Path, PathBuf};
use std::fs::{self, File};
use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::{Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;

/// Create a tar.gz snapshot of the skills directory.
pub fn create_backup(
    skills_dir: &Path,
    backup_dir: &Path,
    reason: Option<&str>,
) -> Result<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let reason_slug = reason.unwrap_or("manual");
    let filename = format!("curator-backup-{}-{}.tar.gz", timestamp, reason_slug);
    let backup_path = backup_dir.join(&filename);

    fs::create_dir_all(backup_dir)
        .with_context(|| format!("Failed to create backup dir: {}", backup_dir.display()))?;

    let tar_gz = File::create(&backup_path)
        .with_context(|| format!("Failed to create backup file: {}", backup_path.display()))?;
    let encoder = GzEncoder::new(tar_gz, Compression::Default);
    let mut archive = tar::Builder::new(encoder);

    archive.append_dir_all(".", skills_dir)
        .with_context(|| format!("Failed to archive directory: {}", skills_dir.display()))?;

    archive.finish()?;
    Ok(backup_path)
}

/// List available backups, sorted newest-first.
pub fn list_backups(backup_dir: &Path) -> Result<Vec<PathBuf>> {
    if !backup_dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<PathBuf> = fs::read_dir(backup_dir)
        .with_context(|| format!("Failed to read backup dir: {}", backup_dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tar.gz"))
        .map(|e| e.path())
        .collect();
    entries.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    Ok(entries)
}

/// Restore skills from a backup archive, returning the path to the
/// previous skills directory (renamed for recovery).
pub fn restore_backup(
    backup_path: &Path,
    skills_dir: &Path,
) -> Result<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let rollback_dir = skills_dir.with_extension(format!("rollback-{}", timestamp));

    // Rename current skills dir to rollback
    if skills_dir.exists() {
        fs::rename(skills_dir, &rollback_dir)
            .context("Failed to rename current skills dir for rollback")?;
    }

    // Extract backup
    let tar_gz = File::open(backup_path)
        .with_context(|| format!("Failed to open backup: {}", backup_path.display()))?;
    let decoder = flate2::read::GzDecoder::new(tar_gz);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(skills_dir)
        .with_context(|| format!("Failed to extract backup to: {}", skills_dir.display()))?;

    Ok(rollback_dir)
}
```

- [ ] **Step 3: Create `curator/archiver.rs` with archive/prune/restore**

```rust
//! Archive, prune, and restore skills.
//! Moves skill directories between active and archive locations.

use std::path::{Path, PathBuf};
use std::fs;
use anyhow::{Context, Result};

/// Archive a skill by moving it from skills_dir to archive_dir.
pub fn archive_skill(
    skill_name: &str,
    skills_dir: &Path,
    archive_dir: &Path,
) -> Result<()> {
    let src = skills_dir.join(skill_name);
    if !src.exists() {
        anyhow::bail!("Skill '{}' not found at {}", skill_name, src.display());
    }
    fs::create_dir_all(archive_dir)?;
    let dst = archive_dir.join(skill_name);
    if dst.exists() {
        fs::remove_dir_all(&dst)?;
    }
    fs::rename(&src, &dst)
        .with_context(|| format!("Failed to archive skill '{}'", skill_name))?;
    Ok(())
}

/// Restore a skill from archive back to active.
pub fn restore_skill(
    skill_name: &str,
    archive_dir: &Path,
    skills_dir: &Path,
) -> Result<()> {
    let src = archive_dir.join(skill_name);
    if !src.exists() {
        anyhow::bail!("Archived skill '{}' not found", skill_name);
    }
    let dst = skills_dir.join(skill_name);
    if dst.exists() {
        anyhow::bail!("A skill named '{}' already exists in active skills", skill_name);
    }
    fs::create_dir_all(skills_dir)?;
    fs::rename(&src, &dst)
        .with_context(|| format!("Failed to restore skill '{}'", skill_name))?;
    Ok(())
}

/// List archived skills.
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
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
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
        let modified = metadata.modified()
            .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
            .unwrap_or(0);

        if metadata.is_dir() && modified < cutoff {
            let name = entry.file_name().to_string_lossy().to_string();
            fs::remove_dir_all(entry.path())?;
            pruned.push(name);
        }
    }
    Ok(pruned)
}
```

- [ ] **Step 4: Create `curator/mod.rs` with the `CuratorEngine` struct**

Add to `curator/mod.rs` (append to the state types from Step 1):

```rust
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::skill_usage::{SkillUsageTracker, UsageRecord};

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
    pub async fn run_review(
        &self,
        dry_run: bool,
        llm_client: Option<&dyn LlmReviewClient>,
    ) -> Result<CuratorReport> {
        let mut state = self.state.write().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let all_usage = self.usage_tracker.list_active()?;
        let agent_created: Vec<&UsageRecord> = all_usage.iter()
            .filter(|r| r.agent_created)
            .collect();

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
                    match archiver::archive_skill(&record.name, &self.skills_dir, &self.archive_dir) {
                        Ok(()) => archived.push(record.name.clone()),
                        Err(e) => errors.push(format!("Failed to archive '{}': {}", record.name, e)),
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
}

/// Trait for LLM-based curator review (for the "LLM review pass" feature).
#[async_trait::async_trait]
pub trait LlmReviewClient: Send + Sync {
    async fn review_skills(&self, skills: &[SkillSummary]) -> Result<Vec<SkillVerdict>>;
}

pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub use_count: u64,
    pub last_used: i64,
}

pub struct SkillVerdict {
    pub skill_name: String,
    pub action: String, // "keep", "archive", "deprecate"
    pub reason: String,
}
```

- [ ] **Step 5: Register `curator` module in `lib.rs`**

Edit `crates/hermes-core/src/lib.rs`:
```rust
pub mod curator;
```

Add the re-exports:
```rust
pub use curator::{
    CuratorEngine, CuratorState, CuratorReport, LlmReviewClient,
    SkillSummary, SkillVerdict,
};
```

- [ ] **Step 6: Verify compilation**

Run: `cargo check --workspace`
Expected: Clean compilation.

- [ ] **Step 7: Write unit tests for curator backup/archive/restore**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_archive_and_restore_skill() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        let archive_dir = tmp.path().join("archive");

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
    }

    #[test]
    fn test_list_archived() {
        let tmp = TempDir::new().unwrap();
        let archive_dir = tmp.path().join("archive");
        fs::create_dir_all(archive_dir.join("skill-a")).unwrap();
        fs::create_dir_all(archive_dir.join("skill-b")).unwrap();

        let list = archiver::list_archived(&archive_dir).unwrap();
        assert_eq!(list, vec!["skill-a", "skill-b"]);
    }

    #[test]
    fn test_prune_archived() {
        let tmp = TempDir::new().unwrap();
        let archive_dir = tmp.path().join("archive");
        let old_dir = archive_dir.join("old-skill");
        fs::create_dir_all(&old_dir).unwrap();
        // Set modified time far in the past
        let old_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(100);
        filetime::set_file_mtime(&old_dir, filetime::FileTime::from_unix_time(old_time)).ok();

        let pruned = archiver::prune_archived(&archive_dir, 1).unwrap();
        assert_eq!(pruned, vec!["old-skill"]);
    }
}
```

- [ ] **Step 8: Run curator tests**

Run: `cargo test --package hermes-core -- test_curator`
Expected: All tests pass.

- [ ] **Step 9: Commit**

```bash
git add crates/hermes-core/src/curator/ crates/hermes-core/src/lib.rs
git commit -m "feat(core): add curator engine with archive/backup/restore"
```

---

### Task 1.2: Curator CLI (Wire to Command Handlers)

**Files:**
- Modify: `crates/hermes-cli/src/cmd_curator.rs` (replace all stubs)
- Test: `crates/hermes-cli/tests/test_curator_cli.rs`

**Dependencies:** Task 1.1 (must be complete first)

- [ ] **Step 1: Rewrite `cmd_curator.rs` to call `CuratorEngine`**

```rust
use anyhow::Result;
use clap::Subcommand;
use hermes_core::config::AppConfig;
use hermes_core::curator::{CuratorEngine, archiver, backup};
use std::sync::Arc;
use hermes_core::skill_usage::SkillUsageTracker;

// (Subcommand enum unchanged - keep the existing CuratorSubcommand)

pub async fn handle_curator_command(config: &AppConfig, cmd: CuratorSubcommand) -> Result<()> {
    let skills_dir = config.skills.skills_dir.clone()
        .unwrap_or_else(|| dirs::data_dir().unwrap().join("hermes").join("skills"));
    let curator_state_path = skills_dir.join(".curator-state.json");
    let usage_tracker = Arc::new(SkillUsageTracker::new(
        skills_dir.join(".usage.json"),
    ));
    let engine = CuratorEngine::new(
        skills_dir.clone(),
        curator_state_path,
        usage_tracker.clone(),
    );
    engine.load_state().await?;

    match cmd {
        CuratorSubcommand::Status => cmd_status(&engine).await,
        CuratorSubcommand::Run { sync, background, dry_run } => {
            cmd_run(&engine, sync, background, dry_run).await
        }
        CuratorSubcommand::Pause => {
            engine.set_paused(true).await?;
            println!("Curator paused.");
            Ok(())
        }
        CuratorSubcommand::Resume => {
            engine.set_paused(false).await?;
            println!("Curator resumed.");
            Ok(())
        }
        CuratorSubcommand::Pin { skill } => {
            usage_tracker.set_pinned(&skill, true)?;
            println!("Skill '{}' pinned.", skill);
            Ok(())
        }
        CuratorSubcommand::Unpin { skill } => {
            usage_tracker.set_pinned(&skill, false)?;
            println!("Skill '{}' unpinned.", skill);
            Ok(())
        }
        CuratorSubcommand::Restore { skill } => {
            let archive_dir = skills_dir.join(".archive");
            archiver::restore_skill(&skill, &archive_dir, &skills_dir)?;
            println!("Skill '{}' restored from archive.", skill);
            Ok(())
        }
        CuratorSubcommand::ListArchived => {
            let archive_dir = skills_dir.join(".archive");
            let archived = archiver::list_archived(&archive_dir)?;
            if archived.is_empty() {
                println!("No archived skills.");
            } else {
                println!("Archived skills:");
                for name in &archived {
                    println!("  - {}", name);
                }
            }
            Ok(())
        }
        CuratorSubcommand::Archive { skill } => {
            let archive_dir = skills_dir.join(".archive");
            archiver::archive_skill(&skill, &skills_dir, &archive_dir)?;
            println!("Skill '{}' archived.", skill);
            Ok(())
        }
        CuratorSubcommand::Prune { days, yes, dry_run } => {
            let archive_dir = skills_dir.join(".archive");
            let threshold = days.unwrap_or(30);
            if !yes {
                println!("Would prune archived skills older than {} days. Pass --yes to confirm.", threshold);
                return Ok(());
            }
            if dry_run {
                println!("[DRY RUN] Would prune archived skills older than {} days.", threshold);
                return Ok(());
            }
            let pruned = archiver::prune_archived(&archive_dir, threshold)?;
            println!("Pruned {} skill(s).", pruned.len());
            Ok(())
        }
        CuratorSubcommand::Backup { reason } => {
            let backup_dir = skills_dir.join(".backups");
            let path = backup::create_backup(&skills_dir, &backup_dir, reason.as_deref())?;
            println!("Backup created: {}", path.display());
            Ok(())
        }
        CuratorSubcommand::Rollback { list, id, yes } => {
            let backup_dir = skills_dir.join(".backups");
            if list {
                let backups = backup::list_backups(&backup_dir)?;
                if backups.is_empty() {
                    println!("No backups available.");
                } else {
                    for b in &backups {
                        println!("  {}", b.file_name().unwrap_or_default().to_string_lossy());
                    }
                }
                return Ok(());
            }
            let backup_id = match id {
                Some(id) => id,
                None => { println!("Use --list to see available backups, then --id <backup-id>."); return Ok(()); }
            };
            if !yes {
                println!("This will replace current skills with the backup. Pass --yes to confirm.");
                return Ok(());
            }
            let backup_path = backup_dir.join(&backup_id);
            if !backup_path.exists() {
                anyhow::bail!("Backup not found: {}", backup_id);
            }
            let rollback_dir = backup::restore_backup(&backup_path, &skills_dir)?;
            println!("Restored from backup. Previous skills moved to: {}", rollback_dir.display());
            Ok(())
        }
    }
}

async fn cmd_status(engine: &CuratorEngine) -> Result<()> {
    let state = engine.state.read().await;
    let active = !state.paused && state.enabled;
    println!("curator: {}", if active { "ACTIVE" } else if state.paused { "PAUSED" } else { "DISABLED" });
    println!("  runs:           {}", state.run_count);
    println!("  last run:       {}", state.last_run_at.map(fmt_ts).unwrap_or_else(|| "never".into()));
    println!("  interval:       {}h", state.interval_hours);
    println!("  stale after:    {}d unused", state.stale_after_days);
    println!("  archive after:  {}d unused", state.archive_after_days);
    if let Some(ref summary) = state.last_run_summary {
        println!("  last summary:   {}", summary);
    }
    Ok(())
}

async fn cmd_run(engine: &CuratorEngine, sync: bool, background: bool, dry_run: bool) -> Result<()> {
    if dry_run {
        let report = engine.run_review(true, None).await?;
        println!("[DRY RUN] {}", report.summary);
        if !report.skills_archived.is_empty() {
            println!("  Would archive:");
            for s in &report.skills_archived { println!("    - {}", s); }
        }
        if !report.skills_stale.is_empty() {
            println!("  Stale (needs attention):");
            for s in &report.skills_stale { println!("    - {}", s); }
        }
        return Ok(());
    }
    if background {
        let eng = engine.clone(); // need Arc-wrapped engine
        tokio::spawn(async move {
            match eng.run_review(false, None).await {
                Ok(report) => println!("Curator run complete: {}", report.summary),
                Err(e) => eprintln!("Curator run failed: {}", e),
            }
        });
        println!("Curator run started in background (PID: process)");
        return Ok(());
    }
    let report = engine.run_review(false, None).await?;
    println!("{}", report.summary);
    if !report.skills_archived.is_empty() {
        println!("  Archived: {}", report.skills_archived.join(", "));
    }
    Ok(())
}
```

Note: The `engine` needs to be wrapped in `Arc` for background spawning. Define a helper type:
```rust
type SharedCuratorEngine = Arc<CuratorEngine>;
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: Clean compilation.

- [ ] **Step 3: Commit**

```bash
git add crates/hermes-cli/src/cmd_curator.rs
git commit -m "feat(cli): wire curator commands to native engine"
```

---

### Task 1.3: Plugins Install (Native Git-Based Install)

**Files:**
- Create: `crates/hermes-cli/src/plugins_install.rs`
- Modify: `crates/hermes-cli/src/cmd_plugins.rs` (replace install stub)
- Add `pub(crate) mod plugins_install;` to `crates/hermes-cli/src/main.rs`
- Test: `crates/hermes-cli/tests/test_plugins_install.rs`

**Dependencies:** None (parallel with Task 1.1, 1.4)

**Python reference:** `hermes_agent/hermes_cli/plugins_cmd.py` (~1587 lines) — uses `git clone` + manifest validation

- [ ] **Step 1: Create `plugins_install.rs` with install logic**

```rust
//! Plugin installation from Git repositories.
//! Ported from hermes-agent/hermes_cli/plugins_cmd.py.

use std::path::{Path, PathBuf};
use std::process::Command;
use anyhow::{Context, Result};

/// Install a plugin from a git URL or owner/repo shorthand.
/// Returns the plugin name (derived from directory name).
pub async fn install_plugin(
    identifier: &str,
    plugins_dir: &Path,
    force: bool,
) -> Result<String> {
    // Resolve identifier to a git URL
    let git_url = resolve_git_url(identifier);
    let plugin_name = derive_plugin_name(identifier);

    let target = plugins_dir.join(&plugin_name);

    if target.exists() {
        if force {
            std::fs::remove_dir_all(&target)
                .with_context(|| format!("Failed to remove existing plugin at {}", target.display()))?;
        } else {
            anyhow::bail!(
                "Plugin '{}' is already installed at {}. Use --force to reinstall.",
                plugin_name, target.display()
            );
        }
    }

    // Clone the repository
    let status = Command::new("git")
        .args(["clone", "--depth", "1", &git_url, &target.to_string_lossy()])
        .status()
        .context("Failed to execute git clone. Is git installed?")?;

    if !status.success() {
        anyhow::bail!("git clone failed for URL: {}", git_url);
    }

    // Validate plugin manifest
    validate_plugin(&target)?;

    Ok(plugin_name)
}

/// Resolve an identifier to a git URL.
/// Supports: full URLs, owner/repo shorthand -> https://github.com/owner/repo
fn resolve_git_url(identifier: &str) -> String {
    if identifier.starts_with("http://")
        || identifier.starts_with("https://")
        || identifier.starts_with("git@")
    {
        identifier.to_string()
    } else if let Some((owner, repo)) = identifier.split_once('/') {
        // Owner/repo shorthand -> GitHub
        format!("https://github.com/{}/{}", owner, repo)
    } else {
        // Assume it's a full name, try GitHub
        format!("https://github.com/{}/{}", identifier, identifier)
    }
}

/// Derive a plugin name from the identifier (last path segment, no .git).
fn derive_plugin_name(identifier: &str) -> String {
    let name = if identifier.ends_with(".git") {
        &identifier[..identifier.len() - 4]
    } else {
        identifier
    };
    name.rsplit_once('/')
        .map(|(_, last)| last.to_string())
        .unwrap_or_else(|| name.to_string())
}

/// Validate that a plugin directory has a valid plugin.yaml manifest.
fn validate_plugin(plugin_dir: &Path) -> Result<()> {
    let manifest_path = plugin_dir.join("plugin.yaml");
    if !manifest_path.exists() {
        // Fallback: check for __init__.py
        let init_path = plugin_dir.join("__init__.py");
        if !init_path.exists() {
            anyhow::bail!(
                "No plugin.yaml or __init__.py found in {}. Is this a valid Hermes plugin?",
                plugin_dir.display()
            );
        }
        return Ok(());
    }

    let content = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;

    // Basic YAML check: must have a name field
    if !content.contains("name:") && !content.contains("name :") {
        anyhow::bail!("plugin.yaml is missing a 'name' field.");
    }

    Ok(())
}

/// Remove an installed plugin.
pub fn remove_plugin(name: &str, plugins_dir: &Path) -> Result<()> {
    let target = plugins_dir.join(name);
    if !target.exists() {
        anyhow::bail!("Plugin '{}' is not installed.", name);
    }
    std::fs::remove_dir_all(&target)
        .with_context(|| format!("Failed to remove plugin '{}'", name))?;
    Ok(())
}

/// List installed plugins.
pub fn list_plugins(plugins_dir: &Path) -> Result<Vec<String>> {
    if !plugins_dir.exists() {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = std::fs::read_dir(plugins_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    Ok(names)
}
```

- [ ] **Step 2: Modify `cmd_plugins.rs` to use the install module**

Replace the `install_plugin` function in `cmd_plugins.rs`:

```rust
async fn install_plugin(
    config: &AppConfig,
    identifier: &str,
    force: bool,
    enable: bool,
) -> Result<()> {
    let dir = plugins_dir(config)?;
    let name = plugins_install::install_plugin(identifier, &dir, force).await?;
    println!("Plugin '{}' installed successfully.", name);
    if enable {
        let marker = dir.join(format!("{}.enabled", name));
        std::fs::write(&marker, "")?;
        println!("Plugin '{}' enabled.", name);
    }
    Ok(())
}
```

- [ ] **Step 3: Add `pub(crate) mod plugins_install;` to `main.rs`**

- [ ] **Step 4: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: Clean compilation.

- [ ] **Step 5: Commit**

```bash
git add crates/hermes-cli/src/plugins_install.rs crates/hermes-cli/src/cmd_plugins.rs crates/hermes-cli/src/main.rs
git commit -m "feat(cli): native plugin install from git repos"
```

---

### Task 1.4: Claw Migration (OpenClaw → Hermes)

**Files:**
- Create: `crates/hermes-cli/src/claw_migrate.rs`
- Modify: `crates/hermes-cli/src/cmd_claw.rs` (replace stubs)
- Add `pub(crate) mod claw_migrate;` to `crates/hermes-cli/src/main.rs`

**Dependencies:** None (parallel with Task 1.1, 1.3)

**Python reference:** `hermes_agent/hermes_cli/setup.py:2885-2950` (detection + migration)

- [ ] **Step 1: Create `claw_migrate.rs`**

```rust
//! Migration from OpenClaw (~/.openclaw) to Hermes format.
//! Ported from hermes-agent/hermes_cli/setup.py:_offer_openclaw_migration

use std::path::PathBuf;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

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
    if dir.is_dir() { Some(dir) } else { None }
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
        let dst = match item.as_str() {
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
            Err(e) => result.errors.push(format!("Failed to migrate '{}': {}", name_str, e)),
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
        return Ok(vec![
            format!("Would move '{}' to '{}'", source.display(), backup_path.display()),
        ]);
    }

    std::fs::rename(source, &backup_path)?;
    Ok(vec![
        format!("Moved '{}' to '{}'", source.display(), backup_path.display()),
        format!("Delete the backup with: rm -rf '{}'", backup_path.display()),
    ])
}
```

- [ ] **Step 2: Modify `cmd_claw.rs` to call migrate/cleanup modules**

- [ ] **Step 3: Add `pub(crate) mod claw_migrate;` to `main.rs`**

- [ ] **Step 4: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: Clean compilation.

- [ ] **Step 5: Commit**

```bash
git add crates/hermes-cli/src/claw_migrate.rs crates/hermes-cli/src/cmd_claw.rs crates/hermes-cli/src/main.rs
git commit -m "feat(cli): native OpenClaw migration and cleanup"
```

---

## Phase 2: Infrastructure & Runtime

### Task 2.1: Gateway Runtime Engine (Fix + Extend)

**Files:**
- Modify: `crates/hermes-cli/src/gateway_runner.rs` (fix config passthrough, add Telegram polling)
- Modify: `crates/hermes-core/src/gateway/mod.rs` (add `start_with_polling`, fix WebhookAdapter)
- Test: `crates/hermes-cli/tests/test_gateway_runner.rs`

**Dependencies:** Task 1.2, 1.3, 1.4 complete (Phase 1 done)

**Python reference:** `hermes_agent/hermes_cli/gateway.py` (~5386 lines) — full process management with systemd/launchd, `gateway/run.py` — main gateway loop

- [ ] **Step 1: Fix `gateway_runner.rs` to pass actual config**

The current `start_gateway` ignores the config and uses `GatewayConfig::default()`. Fix it:

```rust
pub async fn start_gateway(app_config: &AppConfig) -> Result<String> {
    let mut guard = runner().lock().await;

    if guard.is_some() {
        if let Some(gw) = guard.as_ref() {
            if gw.is_running().await {
                return Ok("Gateway is already running.".to_string());
            }
        }
    }

    let gw_config = GatewayConfig {
        telegram_enabled: app_config.gateway.telegram_enabled,
        telegram_token: app_config.gateway.telegram_token.clone(),
        discord_enabled: app_config.gateway.discord_enabled,
        discord_token: app_config.gateway.discord_token.clone(),
        slack_enabled: app_config.gateway.slack_enabled,
        slack_token: app_config.gateway.slack_token.clone(),
        webhooks_enabled: app_config.gateway.webhooks_enabled,
        webhooks_addr: app_config.gateway.webhooks_addr.clone(),
        admins: app_config.gateway.admins.clone(),
    };
    let adapters = build_adapters(&gw_config);

    let mut gateway = Gateway::new(gw_config);
    for adapter in adapters {
        gateway = gateway.with_adapter(adapter);
    }

    let gateway = Arc::new(gateway);
    gateway.start().await.context("Failed to start gateway")?;

    let platform_count = gateway.status().await.len();
    *guard = Some(gateway);

    Ok(format!("Gateway started with {} platform(s).", platform_count))
}
```

- [ ] **Step 2: Add Telegram polling loop**

Add to `crates/hermes-core/src/gateway/mod.rs` — create a `TelegramPoller`:

```rust
/// Polls Telegram for new updates in a loop.
pub struct TelegramPoller {
    adapter: Arc<TelegramAdapter>,
    running: Arc<AtomicBool>,
    interval: Duration,
}

impl TelegramPoller {
    pub fn new(adapter: Arc<TelegramAdapter>) -> Self {
        Self {
            adapter,
            running: Arc::new(AtomicBool::new(false)),
            interval: Duration::from_secs(2),
        }
    }

    pub async fn start(&self, mut tx: mpsc::UnboundedSender<IncomingMessage>) {
        self.running.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut offset: i64 = 0;

        while self.running.load(std::sync::atomic::Ordering::SeqCst) {
            let client = reqwest::Client::new();
            let url = format!("{}/getUpdates", self.adapter.api_url());
            let response = client
                .post(&url)
                .json(&serde_json::json!({
                    "offset": offset,
                    "timeout": 30,
                }))
                .send()
                .await;

            match response {
                Ok(resp) => {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        if let Some(updates) = data["result"].as_array() {
                            for update in updates {
                                if let Some(update_id) = update["update_id"].as_i64() {
                                    offset = update_id + 1;
                                }
                                if let Ok(Some(msg)) = self.adapter.handle_update(update.clone()).await {
                                    let _ = tx.send(msg);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Telegram polling error: {}", e);
                }
            }
        }
    }

    pub fn stop(&self) {
        self.running.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}
```

- [ ] **Step 3: Implement the WebhookAdapter as an axum HTTP server**

Create a webhook listener in `gateway_runner.rs` or a new `gateway_webhooks.rs`:

```rust
/// Start the webhook listener as an axum HTTP server.
/// This receives incoming webhook requests from Telegram/Discord/Slack.
pub async fn start_webhook_listener(
    addr: &str,
    adapters: HashMap<String, Arc<dyn PlatformAdapter>>,
    message_tx: mpsc::UnboundedSender<IncomingMessage>,
) -> Result<()> {
    use axum::{
        Router,
        routing::post,
        extract::State,
        Json,
    };

    let shared_adapters = Arc::new(adapters);

    let app = Router::new()
        .route("/webhook/:platform", post(handle_webhook))
        .with_state(shared_adapters);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn handle_webhook(
    State(adapters): State<Arc<HashMap<String, Arc<dyn PlatformAdapter>>>>,
    axum::extract::Path(platform): axum::extract::Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> &'static str {
    if let Some(adapter) = adapters.get(&platform) {
        match adapter.handle_update(payload).await {
            Ok(Some(msg)) => {
                tracing::info!("Webhook message from {}: {}", platform, msg.content);
                "ok"
            }
            _ => "ignored",
        }
    } else {
        "unknown platform"
    }
}
```

Add `axum` to the workspace Cargo.toml:
```toml
axum = { version = "0.7", features = ["json"] }
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check --workspace`
Expected: Clean compilation.

- [ ] **Step 5: Commit**

```bash
git add crates/hermes-cli/src/gateway_runner.rs crates/hermes-core/src/gateway/mod.rs Cargo.toml
git commit -m "feat(gateway): fix config passthrough, add telegram polling, webhook listener"
```

---

### Task 2.2: ACP Server (Anthropic Client Protocol)

**Files:**
- Create: `crates/hermes-cli/src/acp_server.rs`
- Modify: `crates/hermes-cli/src/cmd_acp.rs` (implement server dispatcher)
- Add `pub(crate) mod acp_server;` to `crates/hermes-cli/src/main.rs`
- Add `axum` dependency (if not already from Task 2.1)

**Dependencies:** Task 2.1 (shares axum pattern, can start after)

**Python reference:** `hermes_agent/acp_adapter/` (11 files) — ACP protocol server for IDE integration

- [ ] **Step 1: Create `acp_server.rs` with minimal ACP server**

ACP (Anthropic Client Protocol) is a JSON-RPC based protocol over stdio. The server:
1. Reads JSON-RPC requests from stdin
2. Dispatches to handlers
3. Writes JSON-RPC responses to stdout

```rust
//! ACP (Anthropic Client Protocol) server.
//! Ported from hermes-agent/acp_adapter/
//! Allows IDEs (VS Code, Zed, JetBrains) to connect to Hermes.

use std::io::{self, BufRead, Write};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// --- JSON-RPC types ---

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

/// Available ACP methods that this server handles.
const ACP_METHODS: &[&str] = &[
    "initialize",
    "shutdown",
    "tools/list",
    "tools/call",
    "notifications/initialized",
];

/// Run the ACP server over stdio.
pub async fn run_acp_server(accept_hooks: bool) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();

    // Signal that the server is ready
    let ready = JsonRpcResponse {
        jsonrpc: "2.0",
        id: Value::Null,
        result: Some(serde_json::json!({
            "serverInfo": {
                "name": "hermes-acp",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "capabilities": {
                "tools": {},
            }
        })),
        error: None,
    };
    let mut output = serde_json::to_string(&ready)?;
    output.push('\n');
    {
        let mut out = stdout.lock();
        out.write_all(output.as_bytes())?;
        out.flush()?;
    }

    // Main loop: read JSON-RPC requests from stdin
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let err_resp = JsonRpcResponse {
                    jsonrpc: "2.0",
                    id: Value::Null,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {}", e),
                        data: None,
                    }),
                };
                write_response(&err_resp)?;
                continue;
            }
        };

        let response = match request.method.as_str() {
            "initialize" => handle_initialize(&request),
            "shutdown" => handle_shutdown(&request),
            "tools/list" => handle_tools_list(&request),
            "tools/call" => handle_tools_call(&request, accept_hooks).await,
            _ => JsonRpcResponse {
                jsonrpc: "2.0",
                id: request.id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32601,
                    message: format!("Method not found: {}", request.method),
                    data: None,
                }),
            },
        };

        write_response(&response)?;
    }

    Ok(())
}

fn handle_initialize(request: &JsonRpcRequest) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id: request.id.clone(),
        result: Some(serde_json::json!({
            "protocolVersion": "0.1.0",
            "capabilities": {
                "tools": {
                    "listChanged": false,
                },
                "experimental": {},
            },
            "serverInfo": {
                "name": "hermes-acp",
                "version": env!("CARGO_PKG_VERSION"),
            },
        })),
        error: None,
    }
}

fn handle_shutdown(request: &JsonRpcRequest) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id: request.id.clone(),
        result: Some(Value::Null),
        error: None,
    }
}

fn handle_tools_list(request: &JsonRpcRequest) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id: request.id.clone(),
        result: Some(serde_json::json!({
            "tools": [
                {
                    "name": "hermes_chat",
                    "description": "Send a message to Hermes and get a response",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "message": {
                                "type": "string",
                                "description": "The message to send to Hermes"
                            },
                            "session_id": {
                                "type": "string",
                                "description": "Optional session ID to continue a conversation"
                            }
                        },
                        "required": ["message"]
                    }
                },
                {
                    "name": "hermes_list_sessions",
                    "description": "List recent conversation sessions",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "limit": {
                                "type": "number",
                                "description": "Max sessions to return"
                            }
                        }
                    }
                }
            ]
        })),
        error: None,
    }
}

async fn handle_tools_call(request: &JsonRpcRequest, _accept_hooks: bool) -> JsonRpcResponse {
    let tool_name = request.params["name"].as_str().unwrap_or("");
    let arguments = &request.params["arguments"];

    match tool_name {
        "hermes_chat" => {
            let message = arguments["message"].as_str().unwrap_or("");
            // TODO: wire into actual Hermes agent
            JsonRpcResponse {
                jsonrpc: "2.0",
                id: request.id.clone(),
                result: Some(serde_json::json!({
                    "content": [
                        {
                            "type": "text",
                            "text": format!("Received: {}", message),
                        }
                    ]
                })),
                error: None,
            }
        }
        "hermes_list_sessions" => {
            JsonRpcResponse {
                jsonrpc: "2.0",
                id: request.id.clone(),
                result: Some(serde_json::json!({
                    "content": [
                        {
                            "type": "text",
                            "text": "Session listing not yet implemented in ACP server",
                        }
                    ]
                })),
                error: None,
            }
        }
        _ => JsonRpcResponse {
            jsonrpc: "2.0",
            id: request.id.clone(),
            result: None,
            error: Some(JsonRpcError {
                code: -32602,
                message: format!("Unknown tool: {}", tool_name),
                data: None,
            }),
        },
    }
}

fn write_response(response: &JsonRpcResponse) -> Result<()> {
    let mut output = serde_json::to_string(response)?;
    output.push('\n');
    let stdout = io::stdout();
    let mut out = stdout.lock();
    out.write_all(output.as_bytes())?;
    out.flush()?;
    Ok(())
}
```

- [ ] **Step 2: Update `cmd_acp.rs`**

```rust
pub async fn handle_acp_command(config: &AppConfig, cmd: AcpSubcommand) -> Result<()> {
    match cmd {
        AcpSubcommand::Server { accept_hooks } => {
            println!("Starting ACP server...");
            crate::acp_server::run_acp_server(accept_hooks).await?;
            Ok(())
        }
    }
}
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: Clean compilation.

- [ ] **Step 4: Commit**

```bash
git add crates/hermes-cli/src/acp_server.rs crates/hermes-cli/src/cmd_acp.rs crates/hermes-cli/src/main.rs
git commit -m "feat(cli): native ACP server over stdio JSON-RPC"
```

---

### Task 2.3: Dashboard Server

**Files:**
- Create: `crates/hermes-cli/src/dashboard_server.rs`
- Modify: `crates/hermes-cli/src/cmd_dashboard.rs` (implement server)
- Add `pub(crate) mod dashboard_server;` to `crates/hermes-cli/src/main.rs`

**Dependencies:** Task 2.1 (shares axum, concurrent start)

**Python reference:** `hermes_agent/hermes_cli/web_server.py` — FastAPI-based dashboard

- [ ] **Step 1: Create `dashboard_server.rs`**

A lightweight axum-based dashboard that serves:
- A status endpoint (`GET /api/status`)
- Static files for the web UI
- A config endpoint

```rust
//! Dashboard HTTP server.
//! Ported from hermes-agent/hermes_cli/web_server.py.
//! Serves a minimal web dashboard for Hermes status monitoring.

use anyhow::{Context, Result};
use axum::{
    Router,
    routing::get,
    response::Json,
    extract::State,
};
use serde::Serialize;
use std::sync::Arc;
use hermes_core::config::{AppConfig, runtime_config};

#[derive(Clone)]
pub struct DashboardState {
    pub start_time: std::time::Instant,
    pub app_config: Arc<AppConfig>,
}

/// Run the dashboard server.
pub async fn run_dashboard(
    config: &AppConfig,
    host: &str,
    port: u16,
    insecure: bool,
) -> Result<()> {
    let state = DashboardState {
        start_time: std::time::Instant::now(),
        app_config: Arc::new(config.clone()),
    };

    let app = Router::new()
        .route("/api/status", get(handle_status))
        .route("/api/config", get(handle_config))
        .layer(
            tower_http::cors::CorsLayer::permissive()
        )
        .with_state(state);

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await
        .with_context(|| format!("Failed to bind to {}", addr))?;

    println!("Dashboard server listening on http://{}", addr);
    if insecure {
        println!("  (running in insecure mode — auth disabled)");
    }

    axum::serve(listener, app).await?;
    Ok(())
}

#[derive(Serialize)]
struct StatusResponse {
    status: String,
    version: String,
    uptime_seconds: u64,
    gateway_running: bool,
    config_path: String,
}

async fn handle_status(
    State(state): State<DashboardState>,
) -> Json<StatusResponse> {
    Json(StatusResponse {
        status: "running".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: state.start_time.elapsed().as_secs(),
        gateway_running: crate::gateway_runner::is_running().await,
        config_path: state.app_config.config_path.display().to_string(),
    })
}

#[derive(Serialize)]
struct ConfigResponse {
    model: String,
    platforms_enabled: Vec<String>,
    skills_dir: String,
    database_path: String,
}

async fn handle_config(
    State(state): State<DashboardState>,
) -> Json<ConfigResponse> {
    let cfg = &state.app_config;
    let mut platforms = Vec::new();
    if cfg.gateway.telegram_enabled { platforms.push("telegram".into()); }
    if cfg.gateway.discord_enabled { platforms.push("discord".into()); }
    if cfg.gateway.slack_enabled { platforms.push("slack".into()); }

    Json(ConfigResponse {
        model: cfg.agent.model.clone(),
        platforms_enabled: platforms,
        skills_dir: state.app_config.skills.skills_dir
            .as_ref().map(|p| p.display().to_string())
            .unwrap_or_default(),
        database_path: cfg.database_path.display().to_string(),
    })
}
```

Add to `Cargo.toml` workspace dependencies:
```toml
tower-http = { version = "0.5", features = ["cors"] }
```

- [ ] **Step 2: Update `cmd_dashboard.rs`**

```rust
pub async fn handle_dashboard_command(config: &AppConfig, cmd: DashboardSubcommand) -> Result<()> {
    match cmd {
        DashboardSubcommand::Server { port, host, no_open, insecure, tui, stop, status } => {
            if stop {
                println!("Dashboard stop not yet implemented (kill the process)");
                return Ok(());
            }
            if status {
                println!("Dashboard status: use the /api/status endpoint when running");
                return Ok(());
            }
            if tui {
                // Launch the existing ratatui TUI instead
                return Err(anyhow::anyhow!(
                    "Use `hermes chat --tui` or `hermes autonomous` for TUI mode"
                ));
            }

            if !no_open {
                let url = format!("http://{}:{}", host, port);
                if let Err(e) = open::that(&url) {
                    tracing::warn!("Failed to open browser: {}", e);
                }
            }

            crate::dashboard_server::run_dashboard(config, &host, port, insecure).await?;
            Ok(())
        }
    }
}
```

Add `open` crate to hermes-cli: `open = "5"` in `Cargo.toml`.

- [ ] **Step 3: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: Clean compilation.

- [ ] **Step 4: Commit**

```bash
git add crates/hermes-cli/src/dashboard_server.rs crates/hermes-cli/src/cmd_dashboard.rs Cargo.toml
git commit -m "feat(cli): native dashboard HTTP server with axum"
```

---

### Task 2.4: MCP Serve (Hermes as MCP Server)

**Files:**
- Create: `crates/hermes-cli/src/mcp_serve.rs`
- Modify: `crates/hermes-cli/src/cmd_mcp.rs` (implement serve)
- Add `pub(crate) mod mcp_serve;` to `crates/hermes-cli/src/main.rs`

**Dependencies:** Task 2.1 (can run concurrently)

**Python reference:** `hermes_agent/mcp_serve.py` (~897 lines) — FastMCP server with 9+ tools

- [ ] **Step 1: Create `mcp_serve.rs`**

This implements Hermes as an MCP server (not client — the existing `hermes_core::mcp` is the client side). The server exposes conversation management via stdio MCP protocol:

```rust
//! Hermes MCP Server — expose messaging conversations as MCP tools.
//! Ported from hermes-agent/mcp_serve.py.
//!
//! Starts a stdio MCP server that lets any MCP client list conversations,
//! read messages, send messages, and poll events.

use std::io::{self, BufRead, Write};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct McpRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct McpResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<McpError>,
}

#[derive(Debug, Serialize)]
struct McpError {
    code: i32,
    message: String,
}

/// Run the MCP server that exposes Hermes tools to MCP clients.
pub async fn run_mcp_serve(_verbose: bool) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let request: McpRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let err_resp = create_error(Value::Null, -32700, &format!("Parse error: {}", e));
                write_mcp_response(&err_resp)?;
                continue;
            }
        };

        let response = match request.method.as_str() {
            "initialize" => handle_mcp_initialize(&request),
            "tools/list" => handle_mcp_tools_list(&request),
            "tools/call" => handle_mcp_tools_call(&request).await,
            "resources/list" => handle_mcp_resources_list(&request),
            "resources/read" => handle_mcp_resources_read(&request),
            "notifications/initialized" => {
                // No response needed for notifications
                continue;
            }
            _ => create_error(
                request.id,
                -32601,
                &format!("Method not found: {}", request.method),
            ),
        };

        write_mcp_response(&response)?;
    }

    Ok(())
}

fn handle_mcp_initialize(request: &McpRequest) -> McpResponse {
    McpResponse {
        jsonrpc: "2.0",
        id: request.id.clone(),
        result: Some(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {
                    "listChanged": false
                },
                "resources": {
                    "subscribe": false
                }
            },
            "serverInfo": {
                "name": "hermes-mcp",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),
        error: None,
    }
}

fn handle_mcp_tools_list(request: &McpRequest) -> McpResponse {
    McpResponse {
        jsonrpc: "2.0",
        id: request.id.clone(),
        result: Some(serde_json::json!({
            "tools": [
                {
                    "name": "conversations_list",
                    "description": "List recent conversations",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "limit": {
                                "type": "number",
                                "description": "Max conversations to return"
                            }
                        }
                    }
                },
                {
                    "name": "messages_send",
                    "description": "Send a message to Hermes and get a response",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "message": {
                                "type": "string",
                                "description": "Message to send"
                            },
                            "session_id": {
                                "type": "string",
                                "description": "Session ID to continue"
                            }
                        },
                        "required": ["message"]
                    }
                },
                {
                    "name": "channels_list",
                    "description": "List available messaging channels",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                }
            ]
        })),
        error: None,
    }
}

async fn handle_mcp_tools_call(request: &McpRequest) -> McpResponse {
    let tool_name = request.params["name"].as_str().unwrap_or("");
    let _arguments = &request.params["arguments"];

    match tool_name {
        "conversations_list" => {
            // TODO: read from session store
            McpResponse {
                jsonrpc: "2.0",
                id: request.id.clone(),
                result: Some(serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": "[]"
                    }]
                })),
                error: None,
            }
        }
        "messages_send" => {
            let message = request.params["arguments"]["message"].as_str().unwrap_or("");
            McpResponse {
                jsonrpc: "2.0",
                id: request.id.clone(),
                result: Some(serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Echo: {}", message),
                    }]
                })),
                error: None,
            }
        }
        "channels_list" => {
            McpResponse {
                jsonrpc: "2.0",
                id: request.id.clone(),
                result: Some(serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": "[]"
                    }]
                })),
                error: None,
            }
        }
        _ => create_error(
            request.id.clone(),
            -32602,
            &format!("Unknown tool: {}", tool_name),
        ),
    }
}

fn handle_mcp_resources_list(request: &McpRequest) -> McpResponse {
    McpResponse {
        jsonrpc: "2.0",
        id: request.id.clone(),
        result: Some(serde_json::json!({
            "resources": []
        })),
        error: None,
    }
}

fn handle_mcp_resources_read(request: &McpRequest) -> McpResponse {
    create_error(request.id.clone(), -32602, "Resource not found")
}

fn create_error(id: Value, code: i32, message: &str) -> McpResponse {
    McpResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(McpError {
            code,
            message: message.to_string(),
        }),
    }
}

fn write_mcp_response(response: &McpResponse) -> Result<()> {
    let mut output = serde_json::to_string(response)?;
    output.push('\n');
    let stdout = io::stdout();
    let mut out = stdout.lock();
    out.write_all(output.as_bytes())?;
    out.flush()?;
    Ok(())
}
```

- [ ] **Step 2: Update `cmd_mcp.rs` `handle_serve()`**

Replace the stub:
```rust
fn handle_serve() -> Result<()> {
    // Launch the MCP server — needs async context
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(crate::mcp_serve::run_mcp_serve(false))?;
    Ok(())
}
```

Better: make `handle_mcp_command` pass through to an async handler. Change the `handle_mcp_command` signature if needed or spawn inside.

- [ ] **Step 3: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: Clean compilation.

- [ ] **Step 4: Commit**

```bash
git add crates/hermes-cli/src/mcp_serve.rs crates/hermes-cli/src/cmd_mcp.rs
git commit -m "feat(cli): native MCP serve as stdio MCP server"
```

---

## Phase 3: Feature Depth & Polish

### Task 3.1: Kanban Multi-Board Support

**Files:**
- Create: `crates/hermes-core/src/kanban/boards.rs`
- Modify: `crates/hermes-core/src/kanban/mod.rs` (add board management)
- Modify: `crates/hermes-core/src/kanban/db.rs` (accept board-scoped paths)
- Modify: `crates/hermes-cli/src/cmd_kanban.rs` (add boards subcommand)

**Dependencies:** None (can start after Phase 2)

**Python reference:** `hermes_agent/hermes_cli/kanban.py:755-875` (boards management), `hermes_agent/hermes_cli/kanban_db.py` (board registry)

- [ ] **Step 1: Create `boards.rs` — board registry**

```rust
//! Board registry — filesystem-backed multi-board support.
//! Ported from hermes-agent/hermes_cli/kanban_db.py boards management.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

const BOARDS_DIR: &str = "boards";
const CURRENT_LINK: &str = "current";
const BOARD_META_FILE: &str = "board.json";
const DEFAULT_BOARD: &str = "default";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardMeta {
    pub slug: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub created_at: i64,
    pub archived: bool,
}

impl BoardMeta {
    pub fn new(slug: &str, name: Option<String>, description: Option<String>) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        Self {
            slug: slug.to_string(),
            name,
            description,
            icon: None,
            color: None,
            created_at: now,
            archived: false,
        }
    }

    pub fn db_path(&self, kanban_root: &Path) -> PathBuf {
        kanban_root.join(BOARDS_DIR).join(&self.slug).join("kanban.db")
    }
}

/// Board registry — manages creation, listing, switching of boards.
pub struct BoardRegistry {
    /// Root kanban directory (e.g., ~/.hermes/kanban/)
    kanban_root: PathBuf,
    /// Boards directory (kanban_root / "boards")
    boards_dir: PathBuf,
}

impl BoardRegistry {
    pub fn new(kanban_root: PathBuf) -> Self {
        let boards_dir = kanban_root.join(BOARDS_DIR);
        Self { kanban_root, boards_dir }
    }

    /// Initialize the board registry (create directories if needed).
    pub fn init(&self) -> Result<()> {
        std::fs::create_dir_all(&self.boards_dir)?;
        // Create the default board if it doesn't exist
        if !self.board_exists(DEFAULT_BOARD) {
            self.create_board(DEFAULT_BOARD, None, None)?;
        }
        // Ensure current pointer exists
        if !self.kanban_root.join(CURRENT_LINK).exists() {
            self.set_current_board(DEFAULT_BOARD)?;
        }
        Ok(())
    }

    /// Create a new board.
    pub fn create_board(
        &self,
        slug: &str,
        name: Option<String>,
        description: Option<String>,
    ) -> Result<BoardMeta> {
        let normed = slug.to_lowercase()
            .replace(' ', "-")
            .replace(|c: char| !c.is_alphanumeric() && c != '-', "");
        if normed.is_empty() {
            anyhow::bail!("Board slug must contain at least one alphanumeric character");
        }

        let board_dir = self.boards_dir.join(&normed);
        std::fs::create_dir_all(&board_dir)?;

        let meta = BoardMeta::new(&normed, name, description);
        let meta_path = board_dir.join(BOARD_META_FILE);
        std::fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;

        Ok(meta)
    }

    /// List all boards.
    pub fn list_boards(&self, include_archived: bool) -> Result<Vec<BoardMeta>> {
        let current = self.get_current_board();
        let mut boards = Vec::new();

        if !self.boards_dir.exists() {
            return Ok(boards);
        }

        for entry in std::fs::read_dir(&self.boards_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let meta_path = entry.path().join(BOARD_META_FILE);
            if !meta_path.exists() {
                continue;
            }
            let content = std::fs::read_to_string(&meta_path)?;
            if let Ok(meta) = serde_json::from_str::<BoardMeta>(&content) {
                if meta.archived && !include_archived {
                    continue;
                }
                boards.push(meta);
            }
        }

        // Sort: current board first, then by name
        boards.sort_by(|a, b| {
            if a.slug == current { std::cmp::Ordering::Less }
            else if b.slug == current { std::cmp::Ordering::Greater }
            else { a.slug.cmp(&b.slug) }
        });

        Ok(boards)
    }

    /// Check if a board exists.
    pub fn board_exists(&self, slug: &str) -> bool {
        self.boards_dir.join(slug).join(BOARD_META_FILE).exists()
    }

    /// Get the currently active board slug.
    pub fn get_current_board(&self) -> String {
        let current_path = self.kanban_root.join(CURRENT_LINK);
        if current_path.exists() {
            std::fs::read_to_string(&current_path)
                .unwrap_or_else(|_| DEFAULT_BOARD.to_string())
                .trim()
                .to_string()
        } else {
            DEFAULT_BOARD.to_string()
        }
    }

    /// Set the current board.
    pub fn set_current_board(&self, slug: &str) -> Result<()> {
        if !self.board_exists(slug) {
            anyhow::bail!("Board '{}' does not exist", slug);
        }
        let current_path = self.kanban_root.join(CURRENT_LINK);
        std::fs::write(&current_path, slug)?;
        Ok(())
    }

    /// Remove (archive) a board.
    pub fn remove_board(&self, slug: &str, archive: bool) -> Result<()> {
        if slug == DEFAULT_BOARD {
            anyhow::bail!("Cannot remove the default board");
        }
        if !self.board_exists(slug) {
            anyhow::bail!("Board '{}' not found", slug);
        }

        if archive {
            let meta_path = self.boards_dir.join(slug).join(BOARD_META_FILE);
            if let Ok(content) = std::fs::read_to_string(&meta_path) {
                if let Ok(mut meta) = serde_json::from_str::<BoardMeta>(&content) {
                    meta.archived = true;
                    std::fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;
                }
            }
        } else {
            std::fs::remove_dir_all(self.boards_dir.join(slug))?;
        }

        // If current board was removed, switch to default
        if self.get_current_board() == slug {
            self.set_current_board(DEFAULT_BOARD)?;
        }

        Ok(())
    }
}
```

- [ ] **Step 2: Update `kanban/db.rs` to accept board path**

Modify `KanbanDb::init` to accept an explicit path (no change needed — it already takes `PathBuf`). The CLI will construct the board-scoped path.

- [ ] **Step 3: Update `kanban/mod.rs`**

```rust
pub mod boards;
pub use boards::{BoardMeta, BoardRegistry};
```

- [ ] **Step 4: Add `boards` subcommand to `cmd_kanban.rs`**

Add to `KanbanSubcommand` enum:
```rust
/// Manage kanban boards (multi-project support)
Boards {
    #[command(subcommand)]
    action: BoardsAction,
},

#[derive(Debug, Clone, Subcommand)]
pub enum BoardsAction {
    /// List all boards
    List {
        /// Include archived boards
        #[arg(long)]
        all: bool,
    },
    /// Create a new board
    Create {
        /// Board slug (used for directory naming)
        slug: String,
        /// Display name
        #[arg(long)]
        name: Option<String>,
        /// Description
        #[arg(long)]
        description: Option<String>,
        /// Switch to this board after creation
        #[arg(long)]
        switch: bool,
    },
    /// Switch to a different board
    Switch {
        /// Board slug
        slug: String,
    },
    /// Show the current board
    Show,
    /// Remove (archive) a board
    Remove {
        /// Board slug
        slug: String,
        /// Permanently delete instead of archiving
        #[arg(long)]
        delete: bool,
    },
}
```

Add handlers that construct `BoardRegistry` and call into it.

- [ ] **Step 5: Verify compilation**

Run: `cargo check --workspace`
Expected: Clean compilation.

- [ ] **Step 6: Commit**

```bash
git add crates/hermes-core/src/kanban/boards.rs crates/hermes-core/src/kanban/mod.rs crates/hermes-cli/src/cmd_kanban.rs
git commit -m "feat(kanban): multi-board support with filesystem registry"
```

---

### Task 3.2: Slash Command Registry (COMMAND_REGISTRY)

**Files:**
- Create: `crates/hermes-cli/src/commands.rs`
- Modify: `crates/hermes-cli/src/main.rs` (wire into `chat_non_tui`)
- Add test: `crates/hermes-cli/tests/test_commands.rs`

**Dependencies:** None (Phase 3, can start independently)

**Python reference:** `hermes_agent/hermes_cli/commands.py` — `COMMAND_REGISTRY` with 50+ `CommandDef` entries

- [ ] **Step 1: Create `commands.rs`**

```rust
//! Slash Command Registry.
//! Ported from hermes-agent/hermes_cli/commands.py COMMAND_REGISTRY.
//! All slash commands are defined here and consumed by the CLI, gateway, etc.

use std::collections::HashMap;

/// A command definition matching the Python CommandDef.
#[derive(Debug, Clone)]
pub struct CommandDef {
    /// Canonical name (without slash)
    pub name: &'static str,
    /// Human-readable description
    pub description: &'static str,
    /// Category: Session, Configuration, Tools & Skills, Info, Exit
    pub category: &'static str,
    /// Alternative names
    pub aliases: &'static [&'static str],
    /// Argument hint shown in help
    pub args_hint: &'static str,
    /// Only available in interactive CLI
    pub cli_only: bool,
    /// Only available in messaging platforms
    pub gateway_only: bool,
}

impl CommandDef {
    pub const fn new(
        name: &'static str,
        description: &'static str,
        category: &'static str,
    ) -> Self {
        Self {
            name,
            description,
            category,
            aliases: &[],
            args_hint: "",
            cli_only: false,
            gateway_only: false,
        }
    }

    pub const fn with_aliases(mut self, aliases: &'static [&'static str]) -> Self {
        self.aliases = aliases;
        self
    }

    pub const fn with_args(mut self, hint: &'static str) -> Self {
        self.args_hint = hint;
        self
    }

    pub const fn cli_only(mut self) -> Self {
        self.cli_only = true;
        self
    }

    pub const fn gateway_only(mut self) -> Self {
        self.gateway_only = true;
        self
    }
}

/// The central command registry — all slash commands are defined here.
pub static COMMAND_REGISTRY: &[CommandDef] = &[
    // ── Session ──
    CommandDef::new("new", "Start a new conversation", "Session")
        .with_aliases(&["n", "clear"]),
    CommandDef::new("reset", "Reset the current conversation", "Session")
        .with_aliases(&["r"]),
    CommandDef::new("continue", "Continue the last conversation", "Session")
        .with_aliases(&["c", "resume"]),
    CommandDef::new("save", "Save the current conversation", "Session")
        .with_aliases(&["export"]),
    CommandDef::new("background", "Run a task in background", "Session")
        .with_args("<prompt>")
        .with_aliases(&["bg"]),
    CommandDef::new("fork", "Fork the conversation from a message", "Session")
        .with_args("<id>"),
    CommandDef::new("history", "Show conversation history", "Session")
        .with_aliases(&["h"]),

    // ── Configuration ──
    CommandDef::new("model", "Switch the active model", "Configuration")
        .with_args("<name>"),
    CommandDef::new("provider", "Switch LLM provider", "Configuration")
        .with_args("<name>"),
    CommandDef::new("config", "View or change configuration", "Configuration")
        .with_args("[key] [value]"),
    CommandDef::new("env", "View or set environment variables", "Configuration")
        .with_args("[key] [value]"),
    CommandDef::new("profile", "Switch or manage profiles", "Configuration")
        .with_args("<name>"),
    CommandDef::new("skin", "Change the CLI theme/skin", "Configuration")
        .with_args("<name>"),

    // ── Tools & Skills ──
    CommandDef::new("skills", "Manage installed skills", "Tools & Skills")
        .with_aliases(&["skill"]),
    CommandDef::new("tools", "List available tools", "Tools & Skills"),
    CommandDef::new("mcp", "Manage MCP servers", "Tools & Skills"),
    CommandDef::new("plugins", "Manage plugins", "Tools & Skills"),
    CommandDef::new("kanban", "Manage kanban tasks", "Tools & Skills")
        .with_aliases(&["k"]),

    // ── Info ──
    CommandDef::new("help", "Show this help message", "Info")
        .with_aliases(&["h", "?"]),
    CommandDef::new("status", "Show system status", "Info"),
    CommandDef::new("memory", "Show or search memories", "Info")
        .with_aliases(&["mem"]),
    CommandDef::new("session", "Show current session info", "Info")
        .with_aliases(&["s"]),
    CommandDef::new("cost", "Show token usage and cost", "Info"),
    CommandDef::new("time", "Show current time", "Info"),

    // ── Exit ──
    CommandDef::new("exit", "Exit the CLI", "Exit")
        .with_aliases(&["quit", "q"]),
];

/// Commands by category (for help display).
pub static COMMANDS_BY_CATEGORY: &[(&str, &[&CommandDef])] = &[
    ("Session", &COMMAND_REGISTRY[0..6]),
    ("Configuration", &COMMAND_REGISTRY[6..12]),
    ("Tools & Skills", &COMMAND_REGISTRY[12..16]),
    ("Info", &COMMAND_REGISTRY[16..22]),
    ("Exit", &COMMAND_REGISTRY[22..23]),
];

/// Build a canonical-name → CommandDef lookup map.
pub fn build_command_map() -> HashMap<&'static str, &'static CommandDef> {
    let mut map = HashMap::new();
    for cmd in COMMAND_REGISTRY {
        map.insert(cmd.name, cmd);
        for alias in cmd.aliases {
            map.insert(alias, cmd);
        }
    }
    map
}

/// Resolve a command name to its canonical name.
pub fn resolve_command(input: &str) -> Option<&'static str> {
    let trimmed = input.trim().trim_start_matches('/');
    let map = build_command_map();
    map.get(trimmed).map(|cmd| cmd.name)
}

/// Format help text for all commands.
pub fn format_help_text() -> String {
    let mut output = String::from("Available commands:\n\n");
    for &(category, commands) in COMMANDS_BY_CATEGORY {
        output.push_str(&format!("  {}:\n", category));
        for cmd in commands {
            let aliases = if cmd.aliases.is_empty() {
                String::new()
            } else {
                format!(" ({})", cmd.aliases.join(", "))
            };
            let args = if cmd.args_hint.is_empty() {
                String::new()
            } else {
                format!(" {}", cmd.args_hint)
            };
            output.push_str(&format!(
                "    /{:<12}{:<20}  {}\n",
                format!("{}{}", cmd.name, args),
                aliases,
                cmd.description
            ));
        }
        output.push('\n');
    }
    output
}
```

- [ ] **Step 2: Wire command registry into `chat_non_tui` in `main.rs`**

Modify the chat loop to resolve slash commands:

```rust
async fn chat_non_tui(config: &AppConfig, system_prompt: Option<&str>) -> Result<()> {
    let mcp_manager = McpManager::new();
    let agent = create_agent_without_events(config, system_prompt, &mcp_manager).await?;

    loop {
        print!("You: ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        if input.is_empty() {
            continue;
        }

        // Check for slash commands
        if input.starts_with('/') {
            let parts: Vec<&str> = input.splitn(2, char::is_whitespace).collect();
            let cmd_name = parts[0];
            let args = parts.get(1).copied().unwrap_or("");

            match cmd_name {
                "/help" | "/h" | "/?" => {
                    println!("{}", commands::format_help_text());
                    continue;
                }
                "/exit" | "/quit" | "/q" => break,
                "/clear" | "/new" | "/n" => {
                    agent.clear_history().await;
                    println!("Conversation cleared. Starting new session.");
                    continue;
                }
                "/history" | "/h" => {
                    // TODO: print conversation history
                    println!("History not yet available in non-TUI mode.");
                    continue;
                }
                "/status" => {
                    println!("Status check...");
                    // TODO: call cmd_status
                    continue;
                }
                "/model" => {
                    if args.is_empty() {
                        println!("Current model: {}", config.agent.model);
                    } else {
                        println!("Model switching not yet supported at runtime.");
                    }
                    continue;
                }
                // Unknown command
                _ => {
                    println!("Unknown command: {}. Type /help for available commands.", cmd_name);
                    continue;
                }
            }
        }

        match agent.run(input.to_string()).await {
            Ok(response) => println!("Assistant: {}\n", response.content),
            Err(error) => eprintln!("Error: {}\n", error),
        }
    }

    Ok(())
}
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: Clean compilation.

- [ ] **Step 4: Commit**

```bash
git add crates/hermes-cli/src/commands.rs crates/hermes-cli/src/main.rs
git commit -m "feat(cli): slash command registry with 23 commands"
```

---

### Task 3.3: RL CLI (Reinforcement Learning Tooling)

**Files:**
- Create: `crates/hermes-cli/src/cmd_rl.rs`
- Modify: `crates/hermes-cli/src/main.rs` (add `Rl` command)
- Create: `crates/hermes-core/src/rl_training/` (optional, for core RL logic)

**Dependencies:** None (Phase 3, independent)

**Python reference:** `hermes_agent/rl_cli.py` (~446 lines) — dedicated RL runner

- [ ] **Step 1: Create `cmd_rl.rs`**

```rust
//! RL Training CLI — run Hermes in RL training mode.
//! Ported from hermes-agent/rl_cli.py.

use anyhow::{Context, Result};
use clap::Subcommand;
use hermes_core::config::AppConfig;

#[derive(Debug, Clone, Subcommand)]
pub enum RlSubcommand {
    /// Start an RL training session
    Run {
        /// Training prompt/instruction
        prompt: String,
        /// Model to use for training
        #[arg(long)]
        model: Option<String>,
        /// Number of training iterations
        #[arg(long, default_value_t = 100)]
        iterations: u32,
    },
    /// List available RL environments
    ListEnvironments,
    /// Check RL environment setup
    Doctor,
}

pub async fn handle_rl_command(config: &AppConfig, cmd: RlSubcommand) -> Result<()> {
    match cmd {
        RlSubcommand::Run { prompt, model, iterations } => {
            run_training(config, &prompt, model, iterations).await
        }
        RlSubcommand::ListEnvironments => list_environments(),
        RlSubcommand::Doctor => doctor_check(config),
    }
}

async fn run_training(
    config: &AppConfig,
    prompt: &str,
    model: Option<String>,
    iterations: u32,
) -> Result<()> {
    let model_name = model.unwrap_or_else(|| config.agent.model.clone());

    println!("Hermes RL Training Session");
    println!("{}", "-".repeat(40));
    println!("Model:      {}", model_name);
    println!("Prompt:     {}", prompt);
    println!("Iterations: {}", iterations);
    println!();

    // Check for required env vars
    let missing_keys = check_rl_env_vars();
    if !missing_keys.is_empty() {
        println!("⚠ Missing required environment variables:");
        for key in &missing_keys {
            println!("  - {}", key);
        }
        println!();
        println!("RL training cannot proceed without these variables.");
        return Ok(());
    }

    println!("Starting RL training with {} iterations...", iterations);
    println!("(RL training environment integration TBD)");

    // TODO: Wire into tinker-atropos or similar RL environment
    // This requires finding the tinker-atropos directory and invoking
    // training scripts or using the rl_training_tool equivalent.

    Ok(())
}

fn list_environments() -> Result<()> {
    println!("Available RL Environments:");
    println!("  - Atropos (tinker-atropos/)");
    println!("  - Custom (via --env-config)");
    Ok(())
}

fn doctor_check(config: &AppConfig) -> Result<()> {
    println!("RL Doctor Check");
    println!("{}", "-".repeat(40));

    let missing = check_rl_env_vars();
    if missing.is_empty() {
        println!("✓ All required environment variables are set.");
    } else {
        for key in &missing {
            println!("✗ Missing: {}", key);
        }
    }

    println!("✓ Configuration loaded from: {}", config.config_path.display());
    println!("✓ Model: {}", config.agent.model);

    Ok(())
}

fn check_rl_env_vars() -> Vec<&'static str> {
    let mut missing = Vec::new();
    for key in &["TINKER_API_KEY", "WANDB_API_KEY", "OPENROUTER_API_KEY"] {
        if std::env::var(key).is_err() {
            missing.push(key);
        }
    }
    missing
}
```

- [ ] **Step 2: Wire `Rl` command in `main.rs`**

Add to the `Commands` enum:
```rust
/// RL training commands
Rl {
    #[command(subcommand)]
    cmd: cmd_rl::RlSubcommand,
},
```

Add dispatch in main:
```rust
Commands::Rl { cmd } => {
    cmd_rl::handle_rl_command(&loaded.config, cmd.clone()).await?;
}
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: Clean compilation.

- [ ] **Step 4: Commit**

```bash
git add crates/hermes-cli/src/cmd_rl.rs crates/hermes-cli/src/main.rs
git commit -m "feat(cli): add RL training subcommand with environment checks"
```

---

## Phase 4: Verification

### Task 4.1: Integration Test — All Commands Are Native

**Files:**
- Script: `scripts/verify_native.sh`
- Test: `crates/hermes-cli/tests/test_no_python_stubs.rs`

**Dependencies:** All Phase 1-3 tasks complete.

- [ ] **Step 1: Create verification script**

```bash
#!/bin/bash
# Verify that no commands reference Python in their output.
# Run after building to check there are no remaining stubs.

set -euo pipefail

echo "=== CLI Parity Verification ==="
echo ""

COMMANDS=(
    "curator status"
    "curator run --dry-run"
    "plugins install --help"
    "plugins list"
    "claw migrate --dry-run"
    "claw cleanup --dry-run"
    "mcp serve --help"
    "acp server --help"
    "dashboard server --help"
    "kanban boards list"
    "kanban boards create test-board"
    "kanban boards switch test-board"
    "kanban boards show"
)

FAILED=0
for cmd in "${COMMANDS[@]}"; do
    OUTPUT=$(hermes $cmd 2>&1 || true)
    if echo "$OUTPUT" | grep -qi "python\|not.*implemented\|information-only\|Python-only\|requires the Python"; then
        echo "❌ STUB DETECTED: hermes $cmd"
        echo "   $OUTPUT" | head -3
        FAILED=$((FAILED + 1))
    else
        echo "✅ PASS: hermes $cmd"
    fi
done

echo ""
if [ $FAILED -eq 0 ]; then
    echo "🎉 All commands are native! Parity achieved."
else
    echo "⚠ $FAILED command(s) still have Python stubs."
fi

exit $FAILED
```

- [ ] **Step 2: Run the verification**

Run: `cargo build --release && ./scripts/verify_native.sh`
Expected: All commands pass with no Python references.

- [ ] **Step 3: Run full test suite**

Run: `cargo test --workspace`
Expected: All tests pass.

- [ ] **Step 4: Run clippy**

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: Clean.

- [ ] **Step 5: Commit**

```bash
git add scripts/verify_native.sh
git commit -m "test: add CLI parity verification script"
```

---

## Parallelization Matrix

```
Phase 1 ──────────────────────────────────────────────────────
  Task 1.1 (Curator Core) ────────┐
  Task 1.3 (Plugins Install) ─────┤  Parallel
  Task 1.4 (Claw Migration) ──────┘
       │
  Task 1.2 (Curator CLI) ─── depends on Task 1.1

Phase 2 ──────────────────────────────────────────────────────
  Task 2.1 (Gateway Engine) ──────┐
  Task 2.2 (ACP Server) ──────────┤  Parallel
  Task 2.3 (Dashboard) ───────────┤
  Task 2.4 (MCP Serve) ───────────┘

Phase 3 ──────────────────────────────────────────────────────
  Task 3.1 (Kanban Boards) ───────┐
  Task 3.2 (Command Registry) ────┤  Parallel
  Task 3.3 (RL CLI) ──────────────┘

Phase 4 ──────────────────────────────────────────────────────
  Task 4.1 (Verification) ─── depends on all above
```

---

## Python Logic References (Delegated Stubs)

The following Python files contain the logic that each task ports:

| Task | Python File(s) | LOC | Key Functions |
|------|----------------|:---:|---------------|
| 1.1 | `agent/curator.py` | ~800 | `load_state()`, `run_review()`, `is_enabled()`, `get_interval_hours()` |
| 1.1 | `agent/curator_backup.py` | ~200 | `create_backup()`, `restore_backup()`, `list_backups()` |
| 1.1 | `tools/skill_usage.py` | ~500 | `agent_created_report()`, `set_pinned()`, `list_active()` |
| 1.2 | `hermes_cli/curator.py` | ~598 | `_cmd_status()`, `_cmd_run()`, `_cmd_pin()`, entire CLI surface |
| 1.3 | `hermes_cli/plugins_cmd.py` | ~1587 | `install_plugin()`, `_resolve_git_executable()`, plugin manifest validation |
| 1.4 | `hermes_cli/setup.py:2885-2950` | ~65 | `_offer_openclaw_migration()`, `scan_openclaw()`, dry-run/preview |
| 2.1 | `hermes_cli/gateway.py` | ~5386 | `start_gateway()`, `stop_gateway()`, process management, systemd/launchd |
| 2.2 | `acp_adapter/` (11 files) | ~2000 | ACP protocol handlers, tool definitions, session management |
| 2.3 | `hermes_cli/web_server.py` | ~3000+ | FastAPI routes, websocket endpoints, PTY bridge |
| 2.4 | `mcp_serve.py` | ~897 | FastMCP server, 9-tool bridge (conversations, messages, events, permissions) |
| 3.1 | `hermes_cli/kanban.py:755-875` | ~120 | `_cmd_boards_*()`, `_dispatch_boards()`, board CRUD |
| 3.1 | `hermes_cli/kanban_db.py` | ~800 | `list_boards()`, `create_board()`, `get_current_board()`, `board_exists()` |
| 3.2 | `hermes_cli/commands.py` | ~500 | `COMMAND_REGISTRY`, `CommandDef`, `resolve_command()`, 50+ commands |
| 3.3 | `rl_cli.py` | ~446 | `run_training()`, `list_environments()`, env var checks, Atropos integration |

---

## Dependency/Cargo Changes Summary

New crate dependencies needed across tasks:

| Crate | Version | Used By | Task |
|-------|---------|---------|:----:|
| `axum` | 0.7 | Gateway webhooks, Dashboard, ACP | 2.1, 2.2, 2.3 |
| `tower-http` | 0.5 (cors) | Dashboard CORS | 2.3 |
| `open` | 5 | Dashboard browser launch | 2.3 |
| `filetime` | 0.2 | Curator archive test timestamps | 1.1 |
| `tempfile` | 3.x | Curator tests | 1.1 |

(Note: `tar` and `flate2` already in workspace dependencies.)
