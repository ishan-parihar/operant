//! Suggested cron jobs — ready-to-run automations the user accepts with one
//! tap. Ported from hermes-agent `cron/suggestions.py` and
//! `hermes_cli/suggestions_cmd.py`.
//!
//! A *suggestion* is a ready-to-run cron job spec that operant surfaces to
//! the user (`operant suggestions list`), who accepts it (creating the real
//! cron job via `CronDb::create_job`) or dismisses it (latched so the same
//! proposal is never re-offered).
//!
//! Sources of suggestions:
//!   * `catalog` — the curated starter automations seeded via
//!     `operant suggestions catalog` (and on first setup),
//!   * `learning` — a future self-improvement review hook: recurring work
//!     noticed by the background review can call [`add`].
//!
//! Storage mirrors the cron database location: `<data_dir>/cron/
//! suggestions.json`, written atomically (tmp file + rename).

use crate::error::Error;
use crate::platform;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// File name of the suggestions store inside the cron directory.
pub const SUGGESTIONS_FILE: &str = "suggestions.json";

/// A ready-to-run cron job spec surfaced to the user.
///
/// Every field is `#[serde(default)]` so the on-disk store stays parseable
/// across schema evolution (a future field addition never bricks old files).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronSuggestion {
    /// Stable store id (assigned on add).
    #[serde(default)]
    pub id: String,
    /// Human title, e.g. "Daily briefing".
    #[serde(default)]
    pub title: String,
    /// One-line description shown under the title.
    #[serde(default)]
    pub description: String,
    /// Origin tag: "catalog", "learning", ...
    #[serde(default)]
    pub source: String,
    /// Dedup key: the same proposal (by this key) is never re-offered.
    #[serde(default)]
    pub dedup_key: String,
    /// Cron schedule expression, e.g. "every 24h" or "0 9 * * *".
    #[serde(default)]
    pub schedule: String,
    /// The prompt/command fed to the agent when the job fires.
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub accepted: bool,
    #[serde(default)]
    pub dismissed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SuggestionFile {
    #[serde(default)]
    suggestions: Vec<CronSuggestion>,
}

/// JSON-backed store for cron suggestions.
pub struct SuggestionStore {
    path: PathBuf,
}

impl SuggestionStore {
    /// Open the default store at `<data_dir>/cron/suggestions.json`.
    pub fn open() -> Result<Self, Error> {
        let dir = platform::operant_data_dir().join("cron");
        fs::create_dir_all(&dir)?;
        Ok(Self {
            path: dir.join(SUGGESTIONS_FILE),
        })
    }

    /// Open a store at an explicit path (used by tests).
    pub fn open_at(path: PathBuf) -> Result<Self, Error> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        Ok(Self { path })
    }

    /// Pending (not accepted, not dismissed) suggestions.
    pub fn pending(&self) -> Result<Vec<CronSuggestion>, Error> {
        Ok(self
            .load()?
            .suggestions
            .into_iter()
            .filter(|s| !s.accepted && !s.dismissed)
            .collect())
    }

    /// Add a suggestion. Returns `false` (and stores nothing) when a
    /// suggestion with the same `dedup_key` already exists in any state —
    /// pending, accepted, or dismissed (latched).
    pub fn add(&self, mut suggestion: CronSuggestion) -> Result<bool, Error> {
        let mut file = self.load()?;
        if file
            .suggestions
            .iter()
            .any(|s| s.dedup_key == suggestion.dedup_key)
        {
            return Ok(false);
        }
        suggestion.id = new_id();
        file.suggestions.push(suggestion);
        self.save(&file)?;
        Ok(true)
    }

    /// Accept a suggestion by id. Returns the accepted suggestion (with its
    /// `accepted` flag set) so the caller can create the real cron job.
    pub fn accept(&self, id: &str) -> Result<Option<CronSuggestion>, Error> {
        let mut file = self.load()?;
        for suggestion in &mut file.suggestions {
            if suggestion.id == id && !suggestion.accepted {
                suggestion.accepted = true;
                let out = suggestion.clone();
                self.save(&file)?;
                return Ok(Some(out));
            }
        }
        Ok(None)
    }

    /// Dismiss a suggestion by id. Latched — it will never be re-offered.
    pub fn dismiss(&self, id: &str) -> Result<bool, Error> {
        let mut file = self.load()?;
        for suggestion in &mut file.suggestions {
            if suggestion.id == id && !suggestion.dismissed {
                suggestion.dismissed = true;
                self.save(&file)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Seed the curated starter automations as pending. Returns how many
    /// were newly added (deduped seeds are skipped).
    pub fn catalog(&self) -> Result<usize, Error> {
        let mut added = 0;
        for suggestion in curated_catalog() {
            if self.add(suggestion)? {
                added += 1;
            }
        }
        Ok(added)
    }

    /// Drop accepted records (housekeeping). Returns the number removed.
    pub fn clear(&self) -> Result<usize, Error> {
        let mut file = self.load()?;
        let before = file.suggestions.len();
        file.suggestions.retain(|s| !s.accepted);
        let removed = before - file.suggestions.len();
        self.save(&file)?;
        Ok(removed)
    }

    fn load(&self) -> Result<SuggestionFile, Error> {
        if !self.path.exists() {
            return Ok(SuggestionFile::default());
        }
        let content = fs::read_to_string(&self.path)?;
        serde_json::from_str(&content).map_err(|e| {
            Error::Agent(format!(
                "corrupt suggestions file {}: {e}",
                self.path.display()
            ))
        })
    }

    fn save(&self, file: &SuggestionFile) -> Result<(), Error> {
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(file)?)?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

/// Curated starter automations (hermes `/suggestions catalog` parity).
pub fn curated_catalog() -> Vec<CronSuggestion> {
    vec![
        CronSuggestion {
            id: String::new(),
            title: "Daily briefing".to_string(),
            description: "Concise briefing of today's priorities, open tasks, and anything time-sensitive.".to_string(),
            source: "catalog".to_string(),
            dedup_key: "catalog_daily_briefing".to_string(),
            schedule: "0 9 * * *".to_string(),
            prompt: "Provide a concise daily briefing: today's priorities, open tasks, deadlines, and anything time-sensitive from sessions, kanban, or memory. Keep it under 200 words.".to_string(),
            accepted: false,
            dismissed: false,
        },
        CronSuggestion {
            id: String::new(),
            title: "Weekly skill audit".to_string(),
            description: "Audit installed skills: flag stale, redundant, or broken ones and suggest consolidations.".to_string(),
            source: "catalog".to_string(),
            dedup_key: "catalog_weekly_skill_audit".to_string(),
            schedule: "every 168h".to_string(),
            prompt: "Audit the installed skills pool: identify stale, redundant, or broken skills and recommend consolidations or removals. Report a concise plan; do not modify anything without approval.".to_string(),
            accepted: false,
            dismissed: false,
        },
        CronSuggestion {
            id: String::new(),
            title: "Session & trajectory hygiene".to_string(),
            description: "Prune old sessions and trajectories to keep the database lean.".to_string(),
            source: "catalog".to_string(),
            dedup_key: "catalog_session_hygiene".to_string(),
            schedule: "every 168h".to_string(),
            prompt: "Review sessions and trajectories older than 30 days and prune the ones with no activity. Do not delete anything newer than 30 days or referenced by a cron job. Report what was removed.".to_string(),
            accepted: false,
            dismissed: false,
        },
        CronSuggestion {
            id: String::new(),
            title: "Kanban stuck-task sweep".to_string(),
            description: "Check boards for tasks stuck in 'running' state and surface blockers.".to_string(),
            source: "catalog".to_string(),
            dedup_key: "catalog_kanban_sweep".to_string(),
            schedule: "every 24h".to_string(),
            prompt: "Check all kanban boards for tasks in 'running' state for over 24 hours. Report each with a one-line status and possible blocker; do not modify any task.".to_string(),
            accepted: false,
            dismissed: false,
        },
        CronSuggestion {
            id: String::new(),
            title: "Monthly memory maintenance".to_string(),
            description: "Consolidate memory facts, remove duplicates, and refresh the user profile.".to_string(),
            source: "catalog".to_string(),
            dedup_key: "catalog_memory_maintenance".to_string(),
            schedule: "every 30d".to_string(),
            prompt: "Run memory maintenance: consolidate duplicate facts, remove stale entries, and refresh the user profile from recent sessions. Report a summary of changes.".to_string(),
            accepted: false,
            dismissed: false,
        },
    ]
}

fn new_id() -> String {
    format!(
        "sug_{}",
        uuid::Uuid::new_v4().to_string()[..8].replace('-', "")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "operant_suggestions_test_{}_{}",
            tag,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir.join("suggestions.json")
    }

    fn sample(dedup_key: &str) -> CronSuggestion {
        CronSuggestion {
            id: String::new(),
            title: "Test job".to_string(),
            description: "A test suggestion".to_string(),
            source: "test".to_string(),
            dedup_key: dedup_key.to_string(),
            schedule: "every 1h".to_string(),
            prompt: "Run a test".to_string(),
            accepted: false,
            dismissed: false,
        }
    }

    #[test]
    fn add_and_pending_round_trip() {
        let path = temp_path("round_trip");
        let store = SuggestionStore::open_at(path).unwrap();
        assert!(store.add(sample("a")).unwrap());
        let pending = store.pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].title, "Test job");
        assert!(pending[0].id.starts_with("sug_"));
    }

    #[test]
    fn dedup_key_latches_across_all_states() {
        let path = temp_path("dedup");
        let store = SuggestionStore::open_at(path).unwrap();
        assert!(store.add(sample("k")).unwrap());
        // Duplicate pending add is refused.
        assert!(!store.add(sample("k")).unwrap());
        // Even after dismiss, the latched key is not re-offered.
        let id = store.pending().unwrap()[0].id.clone();
        assert!(store.dismiss(&id).unwrap());
        assert!(!store.add(sample("k")).unwrap());
        assert!(store.pending().unwrap().is_empty());
    }

    #[test]
    fn accept_returns_spec_and_marks_accepted() {
        let path = temp_path("accept");
        let store = SuggestionStore::open_at(path).unwrap();
        store.add(sample("s")).unwrap();
        let id = store.pending().unwrap()[0].id.clone();
        let accepted = store.accept(&id).unwrap().expect("should accept");
        assert!(accepted.accepted);
        assert_eq!(accepted.schedule, "every 1h");
        assert!(store.accept(&id).unwrap().is_none()); // already accepted
        assert!(store.pending().unwrap().is_empty());
    }

    #[test]
    fn catalog_seeds_are_deduplicated() {
        let path = temp_path("catalog");
        let store = SuggestionStore::open_at(path).unwrap();
        assert_eq!(store.catalog().unwrap(), 5);
        assert_eq!(store.pending().unwrap().len(), 5);
        // Second run adds nothing (dedup latch).
        assert_eq!(store.catalog().unwrap(), 0);
    }

    #[test]
    fn clear_removes_only_accepted() {
        let path = temp_path("clear");
        let store = SuggestionStore::open_at(path).unwrap();
        store.catalog().unwrap();
        let pending = store.pending().unwrap();
        assert!(store.accept(&pending[0].id).unwrap().is_some());
        assert!(store.dismiss(&pending[1].id).unwrap());
        // clear() drops accepted records only.
        assert_eq!(store.clear().unwrap(), 1);
        let remaining = store.pending().unwrap();
        assert_eq!(remaining.len(), 3);
    }

    #[test]
    fn missing_file_is_empty_store() {
        let path = temp_path("missing");
        let store = SuggestionStore::open_at(path).unwrap();
        assert!(store.pending().unwrap().is_empty());
        assert_eq!(store.clear().unwrap(), 0);
    }
}
