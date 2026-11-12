//! Remembering what a lossy method dropped.
//!
//! A lossy method may only run when the full text can be recovered. The
//! store holds that full text under a key the model can be told about; the
//! marker tells the model the key.
//!
//! The key is content-addressed by the caller (the tool-call id), so a
//! second compression of the same result overwrites the same entry rather
//! than growing the store.
//!
//! # Why this store is not the LCM DAG
//!
//! The LCM engine ([`crate::context::lcm`]) is a LOSSLESS DAG of conversation
//! messages. Its only write path is `ingest_turn`, which addresses a node by
//! `(session_id, position, content_hash)` — there is no API that accepts an
//! arbitrary keyed blob — and the DAG has no eviction, so every recall record
//! would grow it without bound. A 25K-token read is ~100 KB and the contract
//! here is a BOUNDED store, so this module keeps its own: a small in-memory
//! ring with an on-disk spill under the operant home.
//!
//! The marker names the key and the tool that resolves it
//! (`evidence_recall "<key>"`); the retrieval side is
//! [`LcmRecall::get`], reached by the `evidence_recall` tool
//! ([`crate::tools::evidence_recall`]). It is deliberately NOT `lcm_recall`:
//! that tool searches the lossless context DAG for earlier CONVERSATION
//! content, while this store holds the pre-compression text of one tool
//! result. A marker naming a tool that cannot resolve its key would promise
//! the model a recovery it cannot get.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// A place the full pre-compression text can be kept.
pub trait RecallStore: Send + Sync {
    /// Store `text` under `key`, returning the key when it was stored.
    ///
    /// Returning `None` means the text was not stored — the caller must then
    /// refuse to run a lossy method.
    fn store(&self, key: &str, text: &str) -> Option<String>;
}

/// A store that keeps nothing, so no lossy method may run.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoRecall;

impl RecallStore for NoRecall {
    fn store(&self, _key: &str, _text: &str) -> Option<String> {
        None
    }
}

/// The marker printed with a compressed result.
///
/// It names the methods that ran, the size before and after, and where the
/// full text is — the model must never have to guess that something was
/// removed.
pub fn marker(key: &str, before: usize, after: usize, applied: &[&str]) -> String {
    let methods = if applied.is_empty() {
        "none".to_string()
    } else {
        applied.join(", ")
    };
    format!(
        "\n[compressed: {methods} — {before} → {after} tokens. Full output: `evidence_recall \"{key}\"`]"
    )
}

/// The key a result's full text is stored under.
pub fn key_for(session_id: Option<&str>, tool_call_id: &str) -> String {
    match session_id {
        Some(session) if !session.is_empty() => format!("{session}:{tool_call_id}"),
        _ => tool_call_id.to_string(),
    }
}

/// The most entries the store keeps. The oldest is evicted first.
const MAX_ENTRIES: usize = 32;

/// The most text bytes the store keeps, across every entry.
const MAX_BYTES: usize = 4 * 1024 * 1024;

/// The most bytes ONE entry may hold.
///
/// A text bigger than this is REFUSED: keeping it would break the bound, and
/// a refused record means the caller must not run a lossy method.
const MAX_ENTRY_BYTES: usize = 1024 * 1024;

/// The most bytes the on-disk spill may hold before it is compacted back to
/// the (already bounded) ring.
const MAX_SPILL_BYTES: u64 = 4 * 1024 * 1024;

/// The recall store: the full, pre-compression text of a lossy result.
///
/// Bounded on BOTH axes — the number of entries and the bytes retained — so
/// a turn that reads twenty 25K-token files cannot grow the store without
/// limit. The oldest entry is evicted first.
pub struct LcmRecall {
    /// Where the on-disk spill lives.
    path: PathBuf,
    /// The retained texts.
    state: Mutex<Ring>,
}

/// The bounded ring of stored texts.
#[derive(Default)]
struct Ring {
    /// `(key, text)`, oldest first.
    entries: VecDeque<(String, String)>,
    /// The text bytes retained across every entry.
    bytes: usize,
}

impl Ring {
    /// Insert the text under `key`, replacing the entry when the key is
    /// already held. The key is content-addressed by the caller, so a
    /// re-compression of the same result REPLACES its record.
    fn upsert(&mut self, key: &str, text: &str) {
        let existing = self.entries.iter().position(|(k, _)| k == key);
        match existing {
            Some(index) => {
                if let Some(entry) = self.entries.get_mut(index) {
                    let old = entry.1.len();
                    entry.1 = text.to_string();
                    self.bytes = self.bytes.saturating_sub(old) + entry.1.len();
                }
            }
            None => {
                self.entries.push_back((key.to_string(), text.to_string()));
                self.bytes += text.len();
            }
        }
    }

    /// Whether the ring still holds `key`.
    fn contains(&self, key: &str) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }

    /// Drop the oldest entries until both bounds hold.
    fn evict_to_bounds(&mut self) {
        while self.entries.len() > MAX_ENTRIES || self.bytes > MAX_BYTES {
            match self.entries.pop_front() {
                Some((_, text)) => self.bytes = self.bytes.saturating_sub(text.len()),
                None => break,
            }
        }
    }
}

impl LcmRecall {
    /// A store that spills under the operant home.
    pub fn new() -> Self {
        Self::with_path(crate::platform::operant_home().join("evidence-recall.jsonl"))
    }

    /// A store that spills to `path`.
    fn with_path(path: PathBuf) -> Self {
        let ring = load_ring(&path);
        LcmRecall {
            path,
            state: Mutex::new(ring),
        }
    }

    /// The text stored under `key`, if it is still retained.
    ///
    /// The retrieval side: the tool that resolves the key the marker names.
    pub fn get(&self, key: &str) -> Option<String> {
        let ring = self.state.lock().unwrap_or_else(|e| e.into_inner());
        ring.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, text)| text.clone())
    }

    /// Where the on-disk spill lives.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Default for LcmRecall {
    fn default() -> Self {
        Self::new()
    }
}

impl RecallStore for LcmRecall {
    fn store(&self, key: &str, text: &str) -> Option<String> {
        if key.is_empty() {
            return None;
        }
        // A text the store cannot hold within its bound is REFUSED, so the
        // caller keeps the original result instead of dropping bytes it
        // cannot recover.
        if text.len() > MAX_ENTRY_BYTES {
            return None;
        }
        let mut ring = self.state.lock().unwrap_or_else(|e| e.into_inner());
        ring.upsert(key, text);
        ring.evict_to_bounds();
        if !ring.contains(key) {
            // The bounds evicted the record that was just written — a marker
            // pointing at a key the store does not hold is worse than none.
            return None;
        }
        // The spill is best-effort: the ring already holds the text, so a
        // failed write costs the record its durability across restarts, not
        // the record itself.
        let _ = spill_append(&self.path, key, text, &ring);
        Some(key.to_string())
    }
}

/// The process-wide recall store.
///
/// The store is a process singleton because the KEY already carries the
/// session id: two sessions never share a key, and one session's records
/// must be reachable from every tool-result chokepoint in the process.
static SHARED: OnceLock<LcmRecall> = OnceLock::new();

/// The process-wide recall store.
pub fn shared() -> &'static LcmRecall {
    SHARED.get_or_init(LcmRecall::default)
}

/// Append one record to the spill file, compacting it when the file outgrows
/// its own bound.
fn spill_append(path: &Path, key: &str, text: &str, ring: &Ring) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = serde_json::json!({ "key": key, "text": text }).to_string();
    {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
    }
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size > MAX_SPILL_BYTES {
        // The ring is the bounded truth, so the spill is rewritten from it —
        // that is what keeps the FILE bounded too.
        spill_compact(path, ring)?;
    }
    Ok(())
}

/// Rewrite the spill from the bounded ring.
fn spill_compact(path: &Path, ring: &Ring) -> std::io::Result<()> {
    let tmp = path.with_extension("jsonl.tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        for (key, text) in &ring.entries {
            let line = serde_json::json!({ "key": key, "text": text }).to_string();
            file.write_all(line.as_bytes())?;
            file.write_all(b"\n")?;
        }
    }
    std::fs::rename(&tmp, path)
}

/// Read the spill back into a ring.
fn load_ring(path: &Path) -> Ring {
    let mut ring = Ring::default();
    let Ok(raw) = std::fs::read_to_string(path) else {
        return ring;
    };
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(key) = value.get("key").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(text) = value.get("text").and_then(|v| v.as_str()) else {
            continue;
        };
        if text.len() > MAX_ENTRY_BYTES {
            continue;
        }
        ring.upsert(key, text);
    }
    ring.evict_to_bounds();
    ring
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marker_names_the_methods_and_the_key() {
        let m = marker("call_1", 1000, 250, &["ansi", "cap"]);
        assert!(m.contains("ansi, cap"), "got {m}");
        assert!(m.contains("1000 → 250"), "got {m}");
        assert!(m.contains("call_1"), "got {m}");
    }

    #[test]
    fn a_marker_with_no_applied_method_says_so() {
        assert!(marker("k", 1, 1, &[]).contains("none"));
    }

    #[test]
    fn the_key_carries_the_session_when_there_is_one() {
        assert_eq!(key_for(Some("s1"), "c1"), "s1:c1");
        assert_eq!(key_for(None, "c1"), "c1");
        assert_eq!(key_for(Some(""), "c1"), "c1");
    }

    #[test]
    fn the_no_recall_store_refuses_to_store() {
        assert!(NoRecall.store("k", "text").is_none());
    }

    fn store_in(dir: &Path) -> LcmRecall {
        LcmRecall::with_path(dir.join("evidence-recall.jsonl"))
    }

    #[test]
    fn a_stored_text_comes_back_under_its_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path());
        assert_eq!(store.store("k", "the full text").as_deref(), Some("k"));
        assert_eq!(store.get("k").as_deref(), Some("the full text"));
    }

    #[test]
    fn the_same_key_replaces_its_entry_rather_than_growing_the_store() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path());
        store.store("k", "first");
        store.store("k", "second");
        assert_eq!(store.get("k").as_deref(), Some("second"));
    }

    #[test]
    fn the_store_is_bounded_by_the_number_of_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path());
        for i in 0..(MAX_ENTRIES + 5) {
            store.store(&format!("k{i}"), "text");
        }
        // The oldest entries are the ones evicted.
        assert!(store.get("k0").is_none(), "the oldest entry must be gone");
        let newest = format!("k{}", MAX_ENTRIES + 4);
        assert!(store.get(&newest).is_some(), "the newest entry must stay");
    }

    #[test]
    fn a_text_too_big_for_the_bound_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path());
        let huge = "x".repeat(MAX_ENTRY_BYTES + 1);
        assert!(store.store("k", &huge).is_none());
        assert!(store.get("k").is_none());
    }

    #[test]
    fn a_record_survives_a_new_store_over_the_same_spill() {
        let dir = tempfile::tempdir().expect("tempdir");
        store_in(dir.path()).store("k", "kept across restarts");
        let reopened = store_in(dir.path());
        assert_eq!(
            reopened.get("k").as_deref(),
            Some("kept across restarts"),
            "the spill is what makes the record survive the process"
        );
    }
}
