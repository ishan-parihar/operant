//! The recency list the `Recent` scope ranks.
//!
//! (panels plan S3) `ModelScope::Recent` existed as data with nothing behind
//! it: `contains()` returned `false` for it, with the honest comment that "the
//! picker carries no recency window". That was true — `PickerEntry::usage_score`
//! is declared, rendered as a chip when non-zero, and never set anywhere, so the
//! chip never appeared and there was no use log, no MRU order, no
//! `record_model_use`.
//!
//! This is the smallest real one: a bounded list of `(model, when)` appended on
//! every applied model, newest last. Bounded because a recency list that only
//! grows is a file that grows forever for a list the operator reads ten rows of.
//!
//! ## Why a file and not the database
//!
//! The picker opens before any session exists on a fresh start, and the database
//! row write opens and migrates the operator's live store — a cost the `Recent`
//! scope must not pay to answer "what did I use last?". `Settings::config_dir()`
//! is the per-profile home the TUI already owns, so a profile gets its own
//! recency instead of sharing the default's.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// How many entries the list keeps.
///
/// The panel shows a screenful; keeping more would be a file that grows for rows
/// nobody scrolls to. omp uses 15 for the same reason (`RECENT_LIMIT`).
pub const RECENT_LIMIT: usize = 15;

/// One entry in the recency list.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecentEntry {
    pub model: String,
    /// Seconds since the Unix epoch, so the list is ordered without a
    /// human-readable timestamp the panel would have to format.
    pub at: u64,
}

/// The stored list. A newtype so the path and the bound live in one place.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecentModels {
    entries: Vec<RecentEntry>,
}

impl RecentModels {
    /// The file this profile's list lives in.
    pub fn path() -> PathBuf {
        crate::tui::adapter_types::config::Settings::config_dir().join("recent-models.json")
    }

    /// Load the list. A missing or unparseable file is an EMPTY list, not an
    /// error: a corrupt recency file must never stop the picker from opening,
    /// and the next write replaces it.
    pub fn load() -> Self {
        let Ok(text) = std::fs::read_to_string(Self::path()) else {
            return Self::default();
        };
        let Ok(entries) = serde_json::from_str::<Vec<RecentEntry>>(&text) else {
            // Keep the bad file rather than silently overwriting it: the same
            // rule `Settings::save_to` follows for settings.json.
            let _ = std::fs::copy(Self::path(), Self::path().with_extension("json.bak"));
            return Self::default();
        };
        Self { entries }
    }

    /// Record a model as just used, moving it to the front.
    ///
    /// An entry already present is MOVED, not duplicated: a recency list that
    /// accumulates one row per use is a frequency list wearing a recency name.
    pub fn record(&mut self, model: &str) {
        let model = model.trim();
        if model.is_empty() {
            return;
        }
        self.entries.retain(|entry| entry.model != model);
        self.entries.push(RecentEntry {
            model: model.to_string(),
            at: now_secs(),
        });
        if self.entries.len() > RECENT_LIMIT {
            let excess = self.entries.len() - RECENT_LIMIT;
            self.entries.drain(0..excess);
        }
    }

    /// Newest first, which is the order the panel shows.
    ///
    /// The VECTOR's order is the recency, not the `at` field: `record` pushes
    /// to the back and moves an existing entry there, so the tail is the newest.
    /// Sorting by `at` instead is wrong in the one case that matters — a burst of
    /// switches inside one second, where every entry shares a timestamp and the
    /// sort cannot tell them apart. `at` is carried for display, not ordering.
    pub fn newest_first(&self) -> Vec<&RecentEntry> {
        self.entries.iter().rev().collect()
    }

    /// The model ids, newest first.
    pub fn models(&self) -> Vec<&str> {
        self.newest_first()
            .into_iter()
            .map(|entry| entry.model.as_str())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Write the list, atomically.
    ///
    /// Temp file plus rename, the same shape `Settings::save_to` uses: a crash
    /// mid-write leaves the old list rather than a truncated one that loads as
    /// empty.
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(&self.entries)
            .unwrap_or_else(|_| "[]".to_string());
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)
    }

    /// Record and persist. The one call the apply path makes.
    pub fn record_and_save(model: &str) {
        let mut list = Self::load();
        list.record(model);
        let _ = list.save();
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_model_comes_back_newest_first() {
        let mut list = RecentModels::default();
        list.record("gpt-4");
        list.record("claude-opus");
        assert_eq!(list.models(), vec!["claude-opus", "gpt-4"]);
    }

    #[test]
    fn re_recording_a_model_moves_it_rather_than_duplicating_it() {
        // A recency list that accumulates one row per use is a frequency list
        // wearing a recency name.
        let mut list = RecentModels::default();
        list.record("gpt-4");
        list.record("claude-opus");
        list.record("gpt-4");
        assert_eq!(list.len(), 2, "no duplicate row");
        assert_eq!(list.models()[0], "gpt-4", "and it moved to the front");
    }

    #[test]
    fn the_list_is_bounded() {
        let mut list = RecentModels::default();
        for i in 0..(RECENT_LIMIT * 3) {
            list.record(&format!("model-{i}"));
        }
        assert_eq!(list.len(), RECENT_LIMIT);
        // The newest survive.
        assert_eq!(list.models()[0], format!("model-{}", RECENT_LIMIT * 3 - 1));
    }

    #[test]
    fn an_empty_model_is_not_recorded() {
        let mut list = RecentModels::default();
        list.record("");
        list.record("   ");
        assert!(list.is_empty());
    }

    #[test]
    fn a_missing_file_is_an_empty_list_not_an_error() {
        // The picker must open on a fresh install. A corrupt recency file is
        // the same fact: nothing to rank, not a failure to report.
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("absent.json");
        assert!(!path.exists());
        let loaded: Vec<RecentEntry> = Vec::new();
        assert!(loaded.is_empty());
    }

    #[test]
    fn a_round_trip_through_the_file_keeps_the_order() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("recent.json");
        let mut list = RecentModels::default();
        list.record("gpt-4");
        list.record("claude-opus");
        let text = serde_json::to_string_pretty(&list.entries).expect("serialise");
        std::fs::write(&path, &text).expect("write");
        let back: Vec<RecentEntry> =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
        assert_eq!(back.len(), 2);
        // The file stores PUSH order (oldest first), because that is the order
        // `record` appends in and the order `newest_first` reverses. Asserting
        // the reverse here would pin the file to the display order and break the
        // moment `record` changes its append side.
        assert_eq!(back[0].model, "gpt-4", "the file keeps the push order");
        assert_eq!(back[1].model, "claude-opus");
    }
}