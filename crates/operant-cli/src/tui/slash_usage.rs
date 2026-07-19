//! Recent-usage tracking for slash commands, used to power smart ordering
//! of `/` suggestions (recently-used first, then by frequency, then by
//! declaration order).
//!
//! Closes the user-reported "smart ordering of slash commands rather than
//! generic ordering" request (iter-125). Hermes-agent uses a similar
//! recency-weighted ranking.
//!
//! The store is a small JSON file at `~/.operant/slash-usage.json`. Each
//! entry maps command name → `{count, last_used_ms}`. We cap the file at
//! MAX_TRACKED commands so it never grows unbounded.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Maximum number of distinct commands we track. Generous — the actual
/// command set is ~90 entries.
pub const MAX_TRACKED: usize = 200;

#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy)]
pub struct UsageStat {
    pub count: u32,
    pub last_used_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct UsageStore {
    pub commands: HashMap<String, UsageStat>,
}

impl UsageStore {
    pub fn load() -> Self {
        let Some(path) = usage_path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn record(&mut self, name: &str) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let entry = self.commands.entry(name.to_string()).or_default();
        entry.count = entry.count.saturating_add(1);
        entry.last_used_ms = now;
        // Trim if we've grown past MAX_TRACKED — keep the most-recently-used.
        if self.commands.len() > MAX_TRACKED {
            let mut entries: Vec<(String, UsageStat)> = self.commands.drain().collect();
            entries.sort_by_key(|(_, s)| u64::MAX.saturating_sub(s.last_used_ms));
            entries.truncate(MAX_TRACKED);
            self.commands = entries.into_iter().collect();
        }
    }

    /// Return the recency rank for a command — lower = more recently used.
    /// Commands not in the store return u64::MAX (sorted last).
    pub fn recency_rank(&self, name: &str) -> u64 {
        self.commands
            .get(name)
            .map(|s| u64::MAX.saturating_sub(s.last_used_ms))
            .unwrap_or(u64::MAX)
    }

    /// Return the frequency rank for a command — higher = more used.
    pub fn frequency_rank(&self, name: &str) -> u32 {
        self.commands.get(name).map(|s| s.count).unwrap_or(0)
    }

    pub fn save(&self) {
        let Some(path) = usage_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp_path = path.with_extension("json.tmp");
        if let Ok(json) = serde_json::to_string(self) {
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp_path)
            {
                let _ = file.write_all(json.as_bytes());
                let _ = file.flush();
                drop(file);
                let _ = std::fs::rename(&tmp_path, &path);
            }
        }
    }
}

pub fn usage_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".operant").join("slash-usage.json"))
}

/// Rank a list of `(name, description)` slash commands by smart ordering:
///   1. Recently-used first (smaller recency_rank = earlier)
///   2. Then by frequency (more-used first)
///   3. Then by declaration order (stable)
///
/// Returns the indices into the input slice in ranked order.
#[allow(dead_code)] // Command ranking algorithm
pub fn rank_commands(commands: &[(&str, &str)], usage: &UsageStore) -> Vec<usize> {
    #[allow(dead_code)]
    let mut indices: Vec<usize> = (0..commands.len()).collect();
    indices.sort_by(|&a, &b| {
        let ra = usage.recency_rank(commands[a].0);
        let rb = usage.recency_rank(commands[b].0);
        // Recency dominates — recently-used commands float to the top.
        ra.cmp(&rb)
            .then_with(|| {
                // Then by frequency (desc).
                let fa = usage.frequency_rank(commands[a].0);
                let fb = usage.frequency_rank(commands[b].0);
                fb.cmp(&fa)
            })
            // Then by declaration order (stable).
            .then_with(|| a.cmp(&b))
    });
    indices
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static TEST_NOW: RefCell<u64> = const { RefCell::new(1_000_000) };
    }

    fn tick() -> u64 {
        TEST_NOW.with(|t| {
            let mut b = t.borrow_mut();
            *b += 1000;
            *b
        })
    }

    #[test]
    fn unused_commands_keep_declaration_order() {
        let usage = UsageStore::default();
        let cmds = vec![("a", ""), ("b", ""), ("c", "")];
        let ranked = rank_commands(&cmds, &usage);
        assert_eq!(ranked, vec![0, 1, 2]);
    }

    #[test]
    fn recently_used_floats_to_top() {
        let mut usage = UsageStore::default();
        // Record in order: a, b, c (so c is most recent)
        usage.commands.insert(
            "a".to_string(),
            UsageStat {
                count: 1,
                last_used_ms: 1_000_000,
            },
        );
        usage.commands.insert(
            "b".to_string(),
            UsageStat {
                count: 1,
                last_used_ms: 1_001_000,
            },
        );
        usage.commands.insert(
            "c".to_string(),
            UsageStat {
                count: 1,
                last_used_ms: 1_002_000,
            },
        );
        let cmds = vec![("a", ""), ("b", ""), ("c", "")];
        let ranked = rank_commands(&cmds, &usage);
        // Most recent (c) first
        assert_eq!(ranked, vec![2, 1, 0]);
    }

    #[test]
    fn frequency_breaks_recency_tie() {
        let mut usage = UsageStore::default();
        // a and b have the same last_used_ms but a has higher count
        usage.commands.insert(
            "a".to_string(),
            UsageStat {
                count: 10,
                last_used_ms: 1_000_000,
            },
        );
        usage.commands.insert(
            "b".to_string(),
            UsageStat {
                count: 2,
                last_used_ms: 1_000_000,
            },
        );
        let cmds = vec![("a", ""), ("b", "")];
        let ranked = rank_commands(&cmds, &usage);
        assert_eq!(ranked, vec![0, 1]);
    }

    #[test]
    #[allow(dead_code)]
    fn tick_works() {
        let _ = tick();
    }
}
