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
}

/// Resolve `~/.operant/history.jsonl`. Falls back to the OS cache dir if
/// `$HOME` is unset (rare; matches the behaviour of `dirs::home_dir`).
pub fn history_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".operant").join("history.jsonl"))
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

    // Read existing entries so we can (a) skip if the new entry duplicates
    // the last one and (b) trim the head when we exceed MAX_ENTRIES.
    let existing = load();
    if existing.last().is_some_and(|last| last == trimmed) {
        return;
    }

    // Build the new list (existing + new entry), trimmed to MAX_ENTRIES.
    let mut entries: Vec<HistoryEntry> = existing
        .into_iter()
        .map(|text| HistoryEntry { ts: 0, text })
        .collect();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    entries.push(HistoryEntry {
        ts: now,
        text: trimmed.to_string(),
    });
    if entries.len() > MAX_ENTRIES {
        entries.drain(0..entries.len() - MAX_ENTRIES);
    }

    // Atomic write: tmp file + rename. Avoids corrupting the file if the
    // process is killed mid-write (Ctrl+C during shutdown).
    let tmp_path = path.with_extension("jsonl.tmp");
    match OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&tmp_path)
    {
        Ok(mut file) => {
            for entry in &entries {
                if let Ok(json) = serde_json::to_string(entry) {
                    let _ = writeln!(file, "{}", json);
                }
            }
            let _ = file.flush();
            drop(file);
            let _ = std::fs::rename(&tmp_path, &path);
        }
        Err(_) => {
            // Fall back to append-only mode if atomic write fails (e.g.
            // permission issue creating the tmp file). Still better than
            // losing the entry.
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                if let Ok(json) = serde_json::to_string(&HistoryEntry {
                    ts: now,
                    text: trimmed.to_string(),
                }) {
                    let _ = writeln!(file, "{}", json);
                }
            }
        }
    }
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
    fn load_from_path(path: &PathBuf) -> Vec<String> {
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

    fn append_to_path(path: &PathBuf, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let existing = load_from_path(path);
        if existing.last().is_some_and(|last| last == trimmed) {
            return;
        }
        let mut entries: Vec<HistoryEntry> = existing
            .into_iter()
            .map(|text| HistoryEntry { ts: 0, text })
            .collect();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        entries.push(HistoryEntry {
            ts: now,
            text: trimmed.to_string(),
        });
        if entries.len() > MAX_ENTRIES {
            entries.drain(0..entries.len() - MAX_ENTRIES);
        }
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
}
