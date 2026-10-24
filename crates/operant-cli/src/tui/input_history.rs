//! Persistent input history — appends every submitted prompt to
//! `~/.operant/history.jsonl` and loads it on TUI launch so up/down arrow
//! cycling works across sessions.
//!
//! Closes the user-reported "up/down must cycle through previously sent
//! messages" request (iter-125). The previous implementation kept history
//! only in memory (`PromptInputState.history: Vec<String>`) and lost it on
//! every restart.
//!
//! Format: one JSON object per line, `{"ts": <unix_ms>, "text": "<input>"}`.
//! We cap the file at MAX_ENTRIES most-recent entries (older entries are
//! trimmed from the head on save).

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Maximum number of entries kept on disk. ~5K entries × ~100 chars avg
/// = ~500 KB worst case, which is fine for a personal-history file.
pub const MAX_ENTRIES: usize = 5_000;

#[derive(Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub ts: u64,
    pub text: String,
    /// (plan 2026-10-24 M7) Project working directory of the submission —
    /// the corpus-quality signal OMP's history.db carries. Absent in files
    /// written before M7; readers treat it as unknown.
    #[serde(default)]
    pub cwd: Option<String>,
    /// (plan 2026-10-24 M7) How many times this DISTINCT prompt has been
    /// submitted. Resubmission bumps the count on the existing row instead
    /// of appending a duplicate (OMP `use_count`).
    #[serde(default = "default_use_count")]
    pub use_count: u32,
}

fn default_use_count() -> u32 {
    1
}

/// Canonical stored form of a prompt (OMP `normalizePrompt`): CRLF/CR
/// folded to LF, trailing whitespace stripped per line, outer trim.
/// Without it a terminal-padded copy of an existing prompt lands as a
/// byte-distinct duplicate row.
fn normalize_prompt(prompt: &str) -> String {
    prompt
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Resolve `<operant home>/history.jsonl`.
pub fn history_path() -> Option<PathBuf> {
    Some(operant_core::platform::operant_home().join("history.jsonl"))
}

/// Load the history file, returning entries oldest → newest. Missing file
/// returns an empty Vec (fresh user). Unparseable lines are silently
/// skipped — we never want a corrupt line to block the TUI.
pub fn load() -> Vec<String> {
    let Some(path) = history_path() else {
        return Vec::new();
    };
    let Ok(file) = File::open(&path) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        // Try parsing as JSON; fall back to treating the line as raw text
        // (back-compat with any hand-edited file).
        if let Ok(entry) = serde_json::from_str::<HistoryEntry>(&line) {
            if !entry.text.is_empty() {
                out.push(entry.text);
            }
        } else if !line.trim().is_empty() {
            out.push(line);
        }
    }
    // Cap in-memory list too — defends against pathological files.
    if out.len() > MAX_ENTRIES {
        out.drain(0..out.len() - MAX_ENTRIES);
    }
    out
}

/// Append a single entry. We dedupe against the most-recent entry so
/// spamming Enter on the same input doesn't fill the file. The file is
/// created if missing. The parent directory is created if missing.
pub fn append(text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    let Some(path) = history_path() else {
        return;
    };
    // Ensure parent dir exists.
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    append_entry(
        &path,
        HistoryEntry {
            ts: now,
            text: trimmed.to_string(),
            cwd: std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned()),
            use_count: 1,
        },
    );
}

/// Shared append path (both the live `append` and the tests, which pass a
/// temp path). OMP semantics (M7): normalize, then DEDUPE BY CONTENT — a
/// resubmission bumps `use_count` and refreshes `ts`/`cwd` on the existing
/// row instead of appending a duplicate. The head-trim keeps the file
/// bounded.
fn append_entry(path: &std::path::Path, entry: HistoryEntry) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut entries = load_entries(path);
    if let Some(existing) = entries
        .iter_mut()
        .find(|e| normalize_prompt(&e.text) == normalize_prompt(&entry.text))
    {
        existing.use_count = existing.use_count.saturating_add(1);
        existing.ts = entry.ts;
        existing.cwd = entry.cwd;
    } else {
        entries.push(entry);
    }
    if entries.len() > MAX_ENTRIES {
        let excess = entries.len() - MAX_ENTRIES;
        entries.drain(0..excess);
    }
    // Atomic write: tmp file + rename (a kill mid-write must not corrupt).
    let tmp_path = path.with_extension("jsonl.tmp");
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&tmp_path)
    {
        for entry in &entries {
            if let Ok(json) = serde_json::to_string(entry) {
                let _ = writeln!(file, "{}", json);
            }
        }
        let _ = file.flush();
        drop(file);
        let _ = std::fs::rename(&tmp_path, path);
    }
}

/// Load raw entries (with cwd/use_count) from a path.
fn load_entries(path: &std::path::Path) -> Vec<HistoryEntry> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<HistoryEntry>(&l).ok())
        .filter(|e| !e.text.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn with_temp_history<F>(f: F)
    where
        F: FnOnce(&PathBuf),
    {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("history.jsonl");
        // Monkey-patch by calling the closure with the temp path; we
        // can't easily override history_path() in unit tests, so we
        // test load/append indirectly via file content.
        f(&path);
    }

    #[test]
    fn load_missing_file_returns_empty() {
        with_temp_history(|path| {
            let entries = load_from_path(path);
            assert!(entries.is_empty());
        });
    }

    #[test]
    fn append_then_load_roundtrip() {
        with_temp_history(|path| {
            append_to_path(path, "first prompt");
            append_to_path(path, "second prompt");
            let entries = load_from_path(path);
            assert_eq!(entries, vec!["first prompt", "second prompt"]);
        });
    }

    #[test]
    fn append_dedupes_consecutive_duplicates() {
        with_temp_history(|path| {
            append_to_path(path, "same");
            append_to_path(path, "same");
            append_to_path(path, "same");
            let entries = load_from_path(path);
            assert_eq!(entries, vec!["same"]);
        });
    }

    #[test]
    fn append_skips_empty_input() {
        with_temp_history(|path| {
            append_to_path(path, "");
            append_to_path(path, "   ");
            append_to_path(path, "\n");
            let entries = load_from_path(path);
            assert!(entries.is_empty());
        });
    }

    // Test helpers that bypass history_path() — we write directly to the
    // temp file.
    fn load_from_path(path: &std::path::Path) -> Vec<String> {
        let Ok(file) = File::open(path) else {
            return Vec::new();
        };
        let reader = BufReader::new(file);
        let mut out = Vec::new();
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<HistoryEntry>(&line) {
                if !entry.text.is_empty() {
                    out.push(entry.text);
                }
            } else if !line.trim().is_empty() {
                out.push(line);
            }
        }
        out
    }

    fn append_to_path(path: &std::path::Path, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        append_entry(
            path,
            HistoryEntry {
                ts: 0,
                text: trimmed.to_string(),
                cwd: None,
                use_count: 1,
            },
        );
    }

    fn load_entries_from(path: &std::path::Path) -> Vec<HistoryEntry> {
        load_entries(path)
    }

    #[test]
    fn resubmission_bumps_use_count_instead_of_appending() {
        with_temp_history(|path| {
            append_to_path(path, "deploy the fleet");
            append_to_path(path, "different prompt");
            append_to_path(path, "deploy the fleet  "); // normalized equal
            let entries = load_entries_from(path);
            assert_eq!(entries.len(), 2, "no duplicate row");
            let bumped = entries
                .iter()
                .find(|e| e.text == "deploy the fleet")
                .expect("row");
            assert_eq!(bumped.use_count, 2, "resubmission counted");
        });
    }

    #[test]
    fn back_compat_reads_old_two_field_lines() {
        with_temp_history(|path| {
            std::fs::write(path, "{\"ts\": 123, \"text\": \"old shape\"}\n").unwrap();
            let texts = load_from_path(path);
            assert_eq!(texts, vec!["old shape"]);
            let entries = load_entries_from(path);
            assert_eq!(entries[0].use_count, 1, "default count for legacy rows");
            assert!(entries[0].cwd.is_none());
        });
    }

    #[test]
    fn normalize_folds_line_trailing_whitespace() {
        with_temp_history(|path| {
            append_to_path(path, "one\ntwo");
            append_to_path(path, "one   \ntwo"); // same after per-line trim
            let entries = load_entries_from(path);
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].use_count, 2);
        });
    }
}
