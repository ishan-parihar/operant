//! Phonon text-to-speech engine (Gradium `phonon-31m-en`).
//!
//! This module wraps the [`ptts`] runtime behind a small, `Mutex`-friendly
//! type so `tools::tts_tool` can hold one engine and call it per turn without
//! knowing anything about `ptts`'s builder, checkpoint resolution, or voice
//! registration.
//!
//! ## Why Phonon
//!
//! `phonon-31m-en` is a 41M-parameter, Apache-2.0, English on-device TTS. On
//! one CPU core it runs ~20x faster than real time with ~5 ms time-to-first
//! byte, because it streams audio frame by frame instead of returning the whole
//! sentence. It replaces Kokoro (82M params, ONNX Runtime, whole-utterance
//! generation) as the local, offline default.
//!
//! ## What is on disk
//!
//! The model is NOT a crate dependency. On first use this module downloads a
//! small GGUF checkpoint into `~/.operant/cache/phonon/<repo>-<rev>/` and
//! loads it from there. Nothing is fetched at build time and no network is
//! touched when the cache is warm:
//!
//!   config.json, tokenizer.json, model.q8.gguf, and one voice file.
//!
//! The cache lives under the operant home, not `$XDG_CACHE_HOME`: every
//! operant store resolves through `platform::operant_home()` so there is one
//! home and no per-subsystem cache fragmentation (see `one_home_resolver.rs`).
//! An install that still has the old `~/.cache/phonon/...` tree is migrated
//! on first use rather than re-downloading 65 MB.
//!
//! `--quant`-style GGUF quants are the point: q8_0 is ~65 MB (near-lossless)
//! and q4k is ~55 MB for size-constrained builds. The ONNX export was
//! deliberately NOT used: `ptts` runs GGUF directly on its own `xn` CPU
//! backend, so there is no ONNX Runtime to pin, download, or panic against.

use std::path::{Path, PathBuf};

use ptts::checkpoint::read_config;
use ptts::preprocess::Lang;
use ptts::synth::{Quant, Synth};

/// The Hugging Face repo the checkpoint is downloaded from.
const MODEL_REPO: &str = "gradium/phonon-31m-en";

/// The revision every file is pinned to. A default install must be
/// reproducible, and a moving `main` would silently change the voice.
const MODEL_REVISION: &str = "92741c5b07d085ff840e26fea8ce43110b87852f";

/// Model files fetched into the cache directory, in order.
const MODEL_FILES: &[&str] = &[
    "config.json",
    "tokenizer.json",
    "model.q8.gguf",
    "default-voice.safetensors",
];

/// Expected size of `model.q8.gguf` in bytes, used to reject a truncated
/// download. A GET that dies mid-stream (or a captive-portal error page
/// captured with a 200) would otherwise be written as a short file that
/// `exists()` accepts as complete and `ptts` then fails to load.
const MODEL_FILE_SIZES: &[(&str, u64)] = &[
    ("config.json", 4_634),
    ("tokenizer.json", 277_967),
    ("model.q8.gguf", 64_512_352),
    ("default-voice.safetensors", 256_136),
];

/// The quant `ptts` loads the GGUF weights as. q8_0 keeps quality
/// effectively lossless on a 31M model while halving the download versus
/// f32; q4k (55 MB) is the escape hatch for disk-constrained builds.
const QUANT: Quant = Quant::Q80;

/// A loaded Phonon model, ready to synthesize.
///
/// `ptts::Synth` is `!Sync` (it owns a streaming decoder thread and mutable
/// model state), so every caller drives it through a `Mutex`. The type is
/// cheap to move behind an `Arc<Mutex<Option<…>>>`: it is one boxed
/// pipeline, and synthesis borrows it mutably.
pub struct PhononSynth {
    inner: Synth,
}

impl PhononSynth {
    /// Synthesize `text` to `f32` samples in `[-1.0, 1.0]`, mono, at
    /// [`Self::sample_rate`].
    ///
    /// `ptts` generates the waveform whole; the samples are ready for a WAV
    /// writer. `voice` names a registered voice (`default` unless overridden).
    pub fn synthesize(&self, text: &str, voice: Option<&str>) -> ptts::Result<Vec<f32>> {
        let opts = ptts::synth::SpeechOptions {
            voice: voice.map(|v| v.to_string()),
            ..Default::default()
        };
        self.inner.say_with(text, &opts)
    }

    /// The sample rate of the samples `synthesize` returns, in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
}

/// Write `samples` as a 16-bit mono WAV at `sample_rate` into `out`.
///
/// Shared by every local provider in this module: Phonon hands back `f32`
/// in `[-1.0, 1.0]`, and this is the one place that conversion happens.
pub fn write_wav(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer =
            hound::WavWriter::new(&mut cursor, spec).map_err(|e| format!("WAV writer: {e}"))?;
        for &sample in samples {
            let s = (sample * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            writer
                .write_sample(s)
                .map_err(|e| format!("WAV write: {e}"))?;
        }
        writer
            .finalize()
            .map_err(|e| format!("WAV finalize: {e}"))?;
    }
    Ok(cursor.into_inner())
}

/// The cache directory this engine keeps its checkpoint in.
///
/// `~/.operant/cache/phonon/<repo>-<rev>/`. It lives under the operant home so
/// every operant store resolves through `platform::operant_home()` and there
/// is one home, not a per-subsystem `$XDG_CACHE_HOME` tree. The revision is
/// part of the path so bumping `MODEL_REVISION` invalidates the cache instead
/// of silently reusing the previous weights.
pub fn cache_dir() -> Option<PathBuf> {
    let home = crate::platform::operant_home();
    Some(home.join("cache").join("phonon").join(format!(
        "{}-{}",
        repo_name(),
        &MODEL_REVISION[..12]
    )))
}

/// The pre-iter-2524 cache location, read only to migrate an existing tree.
fn legacy_cache_dir() -> Option<PathBuf> {
    let base = dirs::cache_dir().or_else(dirs::home_dir)?;
    Some(base.join("phonon").join(repo_name()))
}

fn repo_name() -> &'static str {
    MODEL_REPO
}

/// Expected byte length of one model file, if it is pinned.
fn expected_size(file: &str) -> Option<u64> {
    MODEL_FILE_SIZES
        .iter()
        .find(|(name, _)| *name == file)
        .map(|(_, size)| *size)
}

/// Whether a cached file is present AND the right size. A short file is a
/// truncated download, not a cache hit: `ptts` would fail on it with
/// "failed to fill whole buffer", so the only safe answer is to re-fetch.
fn file_is_complete(dir: &Path, file: &str) -> bool {
    let path = dir.join(file);
    let Ok(meta) = std::fs::metadata(&path) else {
        return false;
    };
    match expected_size(file) {
        Some(want) => meta.len() == want,
        // Unpinned file: existence is all we can check.
        None => true,
    }
}

/// Whether the checkpoint is already present in the cache, complete.
pub fn is_cached() -> bool {
    let Some(dir) = cache_dir() else {
        return false;
    };
    MODEL_FILES.iter().all(|f| file_is_complete(&dir, f))
}

/// Download any missing checkpoint files into the cache, then load them.
///
/// Every file is fetched from a pinned revision on the Hugging Face Hub, so a
/// warm cache never touches the network and a cold one is reproducible. The
/// download is a plain `reqwest` GET of a `resolve/<rev>/<file>` URL; there
/// is no Hub client to configure.
pub async fn load_or_download() -> Result<PhononSynth, String> {
    let dir = cache_dir().ok_or_else(|| "no operant home for the Phonon model".to_string())?;
    download_missing(&dir).await?;

    let config = read_config(dir.join("config.json")).map_err(|e| e.to_string())?;
    let synth = Synth::builder(config, dir.join("model.q8.gguf"), Lang::En)
        .quant(QUANT)
        .tokenizer_file(dir.join("tokenizer.json"))
        .add_voice("default", dir.join("default-voice.safetensors"))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(PhononSynth { inner: synth })
}

/// Fetch every file in [`MODEL_FILES`] that is missing or truncated in `dir`,
/// then report whether the cache is complete.
///
/// Shared with `operant setup`, which must not carry its own downloader: two
/// downloaders in one repo is how a pinned revision and a moving `main` come
/// to disagree (audit finding on iter-2513).
pub async fn ensure_downloaded() -> Result<bool, String> {
    let dir = cache_dir().ok_or_else(|| "no operant home for the Phonon model".to_string())?;
    download_missing(&dir).await?;
    Ok(MODEL_FILES.iter().all(|f| file_is_complete(&dir, f)))
}

/// Fetch every file in [`MODEL_FILES`] that is missing or truncated in `dir`.
///
/// Each file is downloaded to `<file>.part` and renamed into place only after
/// it matches its expected size, so a failed or truncated download can never
/// masquerade as a complete one. Any stale `.part` from a previous interrupted
/// run is removed before the retry.
async fn download_missing(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {dir:?}: {e}"))?;
    migrate_legacy_cache(dir)?;
    let client = reqwest::Client::new();
    for file in MODEL_FILES {
        if file_is_complete(dir, file) {
            continue;
        }
        let url = format!("https://huggingface.co/{MODEL_REPO}/resolve/{MODEL_REVISION}/{file}");
        let tmp = dir.join(format!("{file}.part"));
        // A stale partial from an interrupted run must not be mistaken for
        // a fresh download; remove it before fetching.
        let _ = std::fs::remove_file(&tmp);
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("download {file}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("download {file}: HTTP {}", resp.status()));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("download {file}: {e}"))?;
        if let Some(want) = expected_size(file)
            && bytes.len() as u64 != want
        {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!(
                "download {file}: got {} bytes, expected {want} (truncated?)",
                bytes.len()
            ));
        }
        std::fs::write(&tmp, &bytes).map_err(|e| format!("write {file}: {e}"))?;
        std::fs::rename(tmp, dir.join(file)).map_err(|e| format!("install {file}: {e}"))?;
    }
    Ok(())
}

/// Move a pre-iter-2524 `~/.cache/phonon/<repo>/` tree into the operant home.
///
/// The cache used to live outside the operant home. Rather than re-download
/// 65 MB for every existing install, copy any complete file across once. A
/// file that is missing or truncated in the old location is left alone and
/// re-fetched normally.
fn migrate_legacy_cache(dir: &Path) -> Result<(), String> {
    let Some(old) = legacy_cache_dir() else {
        return Ok(());
    };
    if old == dir || !old.is_dir() {
        return Ok(());
    }
    for file in MODEL_FILES {
        if file_is_complete(dir, file) {
            continue;
        }
        if !file_is_complete(&old, file) {
            continue;
        }
        let from = old.join(file);
        let to = dir.join(file);
        std::fs::copy(&from, &to).map_err(|e| format!("migrate {file}: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_dir_is_under_the_operant_home_with_the_revision() {
        let Some(dir) = cache_dir() else {
            return;
        };
        let s = dir.to_string_lossy();
        assert!(s.contains("phonon"), "cache dir must name phonon: {s}");
        assert!(
            s.contains("cache"),
            "cache dir must be under a cache/ subdir: {s}"
        );
        assert!(
            s.contains(&MODEL_REVISION[..12]),
            "cache dir must carry the revision so a bump invalidates it: {s}"
        );
        // The cache resolves through the operant home so there is one home,
        // not a per-subsystem $XDG_CACHE_HOME tree.
        assert!(
            s.contains(".operant") || s.contains(".hermes"),
            "cache dir must be under the operant home: {s}"
        );
    }

    #[test]
    fn a_truncated_file_is_not_a_cache_hit() {
        // A short file is a failed download, not a complete one: `ptts` would
        // fail on it with "failed to fill whole buffer".
        let tmp = std::env::temp_dir().join(format!("phonon-trunc-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        for file in MODEL_FILES {
            let want = expected_size(file).unwrap();
            // Write one byte short: exists() would say yes, file_is_complete must not.
            std::fs::write(tmp.join(file), vec![0u8; (want - 1) as usize]).unwrap();
            assert!(!file_is_complete(&tmp, file), "{file} must be rejected");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn repo_name_is_the_pinned_model() {
        assert_eq!(repo_name(), "gradium/phonon-31m-en");
    }
}
