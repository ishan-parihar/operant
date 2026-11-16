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
//! small GGUF checkpoint into `~/.cache/phonon/<name>/` and loads it from
//! there. Nothing is fetched at build time and no network is touched when the
//! cache is warm:
//!
//!   config.json, tokenizer.json, model.q8.gguf, and one voice file.
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

    /// The names of the voices this engine can select with
    /// [`Self::synthesize`]'s `voice` argument.
    pub fn voices(&self) -> Vec<String> {
        self.inner.voices()
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
/// `~/.cache/phonon/<repo-name>/`, with `$XDG_CACHE_HOME` honoured on Linux
/// and the platform cache dir elsewhere. This is a MODEL cache, not an
/// operant-home store, so it deliberately does NOT resolve through
/// `platform::operant_home()` — see `one_home_resolver.rs`, which carries a
/// matching allowance.
pub fn cache_dir() -> Option<PathBuf> {
    let base = dirs::cache_dir().or_else(dirs::home_dir)?;
    Some(base.join("phonon").join(repo_name()))
}

fn repo_name() -> &'static str {
    MODEL_REPO
}

/// Whether the checkpoint is already present in the cache.
pub fn is_cached() -> bool {
    let Some(dir) = cache_dir() else {
        return false;
    };
    MODEL_FILES.iter().all(|f| dir.join(f).exists())
}

/// Download any missing checkpoint files into the cache, then load them.
///
/// Every file is fetched from a pinned revision on the Hugging Face Hub, so a
/// warm cache never touches the network and a cold one is reproducible. The
/// download is a plain `reqwest` GET of a `resolve/<rev>/<file>` URL; there
/// is no Hub client to configure.
pub async fn load_or_download() -> Result<PhononSynth, String> {
    let dir = cache_dir().ok_or_else(|| "no cache directory for the Phonon model".to_string())?;
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

/// Fetch every file in [`MODEL_FILES`] that is missing from `dir`.
async fn download_missing(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {dir:?}: {e}"))?;
    let client = reqwest::Client::new();
    for file in MODEL_FILES {
        let dest = dir.join(file);
        if dest.exists() {
            continue;
        }
        let url = format!("https://huggingface.co/{MODEL_REPO}/resolve/{MODEL_REVISION}/{file}");
        let tmp = dir.join(format!("{file}.part"));
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
        std::fs::write(&tmp, &bytes).map_err(|e| format!("write {file}: {e}"))?;
        std::fs::rename(&tmp, &dest).map_err(|e| format!("install {file}: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_dir_is_under_the_model_cache_not_the_operant_home() {
        let Some(dir) = cache_dir() else {
            return;
        };
        let s = dir.to_string_lossy();
        assert!(s.contains("phonon"), "cache dir is {s}");
        assert!(
            !s.contains(".operant"),
            "cache dir must not be the operant home: {s}"
        );
    }

    #[test]
    fn repo_name_is_the_pinned_model() {
        assert_eq!(repo_name(), "gradium/phonon-31m-en");
    }
}
