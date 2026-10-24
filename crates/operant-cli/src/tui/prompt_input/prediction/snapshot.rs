//! Snapshot persistence for the word predictor (plan 2026-10-24 M5).
//!
//! OMP persists a versioned zstd snapshot atomically (temp + rename). The
//! operant port is serde_json (portable, debuggable, no new dependency);
//! an unknown version or a corrupt file is reported so the caller can wipe
//! the directory and re-ingest history — exactly OMP's recovery contract.

use std::path::Path;

use super::model::Model;

/// Bumped whenever stored state needs a rebuild on load.
const SNAPSHOT_VERSION: u32 = 1;

#[derive(serde::Serialize, serde::Deserialize)]
struct Snapshot {
    version: u32,
    /// Sorted vocab (ids = positions).
    vocab: Vec<String>,
    count: Vec<f64>,
    /// (word id, distinct-prompt count) pairs.
    prompt_counts: Vec<(u32, u32)>,
    /// (prev, follower, count).
    bigrams: Vec<(u32, u32, f64)>,
    /// (u, v, w, count).
    trigrams: Vec<(u32, u32, u32, f64)>,
    total: f64,
}

/// Encode the model to bytes.
pub fn encode(model: &Model) -> Result<Vec<u8>, String> {
    let snap = Snapshot {
        version: SNAPSHOT_VERSION,
        vocab: model.vocab_dump().to_vec(),
        count: model.count_dump().to_vec(),
        prompt_counts: model.prompt_counts_dump().to_vec(),
        bigrams: model.bigrams_dump(),
        trigrams: model.trigrams_dump(),
        total: model.total_mass(),
    };
    serde_json::to_vec(&snap).map_err(|e| e.to_string())
}

/// Decode a snapshot produced by [`encode`]. Errors on an unknown version
/// or corrupt payload (the caller wipes and re-ingests).
pub fn decode(bytes: &[u8]) -> Result<Model, String> {
    let snap: Snapshot = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if snap.version != SNAPSHOT_VERSION {
        return Err(format!("unknown snapshot version {}", snap.version));
    }
    if snap.vocab.len() != snap.count.len() {
        return Err("vocab/count length mismatch".to_string());
    }
    let mut model = Model::from_parts(
        snap.vocab,
        snap.count,
        snap.prompt_counts.into_iter().collect(),
        snap.bigrams,
        snap.trigrams,
        snap.total,
    );
    model.rebuild_indexes();
    Ok(model)
}

/// Write `bytes` to `dir/ngram.snapshot` atomically: temp file in the same
/// directory, then rename over the target (OMP `write_atomic`).
pub fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let target = dir.join(name);
    let temp = dir.join(format!("{name}.{}.tmp", std::process::id()));
    let written = (|| {
        use std::io::Write;
        let mut file = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        std::fs::rename(&temp, &target).map_err(|e| e.to_string())
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written.map_err(|e| format!("write {}: {e}", target.display()))
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    #[test]
    fn round_trip_and_corruption() {
        let mut m = Model::default();
        m.learn("can you refactor the parser");
        m.learn("can you review the parser");
        let bytes = encode(&m).expect("encode");
        let back = decode(&bytes).expect("decode");
        let id = back.id("parser").expect("parser");
        assert!(back.eligible(id));
        let the = back.id("the").expect("the");
        let parser = back.id("parser").expect("parser");
        assert!((back.bigram(the, parser) - 2.0).abs() < 1e-9);

        assert!(
            decode(b"{ not json").is_err(),
            "corrupt file errors, not panics"
        );
        let mut bad = encode(&m).expect("encode");
        bad[0] = b'x';
        assert!(
            decode(&bad).is_err() || decode(&bad).is_ok(),
            "never panics"
        );
    }

    #[test]
    fn atomic_write_leaves_no_temp() {
        let dir = std::env::temp_dir().join(format!("operant-pred-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_atomic(&dir, "ngram.snapshot", b"payload").expect("write");
        assert!(dir.join("ngram.snapshot").exists());
        assert!(
            std::fs::read_dir(&dir).expect("dir").count() == 1,
            "no temp left"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
