//! `tts` configuration surface — extracted verbatim from the
//! former schema.rs monolith (dedup pass). Placement is navigational;
//! every item is re-exported from `schema::`.

use anyhow::Result;
use operant_macros::Configurable;
#[cfg(feature = "schema-export")]
use serde::{Deserialize, Serialize};

use super::*;

/// Text-to-Speech configuration (`[tts]`).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "tts"]
pub struct TtsConfig {
    /// Enable TTS synthesis.
    #[serde(default)]
    pub enabled: bool,
    /// Default TTS provider (`"openai"`, `"elevenlabs"`, `"google"`, `"edge"`).
    #[serde(default = "default_tts_provider")]
    pub default_provider: String,
    /// Default voice ID passed to the selected provider.
    #[serde(default = "default_tts_voice")]
    pub default_voice: String,
    /// Default audio output format (`"mp3"`, `"opus"`, `"wav"`).
    #[serde(default = "default_tts_format")]
    pub default_format: String,
    /// Maximum input text length in characters (default 4096).
    #[serde(default = "default_tts_max_text_length")]
    pub max_text_length: usize,
    /// OpenAI TTS provider configuration (`[tts.openai]`).
    #[serde(default)]
    #[nested]
    pub openai: Option<OpenAiTtsConfig>,
    /// ElevenLabs TTS provider configuration (`[tts.elevenlabs]`).
    #[serde(default)]
    #[nested]
    pub elevenlabs: Option<ElevenLabsTtsConfig>,
    /// Google Cloud TTS provider configuration (`[tts.google]`).
    #[serde(default)]
    #[nested]
    pub google: Option<GoogleTtsConfig>,
    /// Edge TTS provider configuration (`[tts.edge]`).
    #[serde(default)]
    #[nested]
    pub edge: Option<EdgeTtsConfig>,
    /// Piper TTS provider configuration (`[tts.piper]`).
    #[serde(default)]
    #[nested]
    pub piper: Option<PiperTtsConfig>,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_provider: default_tts_provider(),
            default_voice: default_tts_voice(),
            default_format: default_tts_format(),
            max_text_length: default_tts_max_text_length(),
            openai: None,
            elevenlabs: None,
            google: None,
            edge: None,
            piper: None,
        }
    }
}

/// OpenAI TTS provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "tts.openai"]
pub struct OpenAiTtsConfig {
    /// API key for OpenAI TTS.
    #[serde(default)]
    #[secret]
    pub api_key: Option<String>,
    /// Model name (default `"tts-1"`).
    #[serde(default = "default_openai_tts_model")]
    pub model: String,
    /// Playback speed multiplier (default `1.0`).
    #[serde(default = "default_openai_tts_speed")]
    pub speed: f64,
}

/// ElevenLabs TTS provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "tts.elevenlabs"]
pub struct ElevenLabsTtsConfig {
    /// API key for ElevenLabs.
    #[serde(default)]
    #[secret]
    pub api_key: Option<String>,
    /// Model ID (default `"eleven_monolingual_v1"`).
    #[serde(default = "default_elevenlabs_model_id")]
    pub model_id: String,
    /// Voice stability (0.0-1.0, default `0.5`).
    #[serde(default = "default_elevenlabs_stability")]
    pub stability: f64,
    /// Similarity boost (0.0-1.0, default `0.5`).
    #[serde(default = "default_elevenlabs_similarity_boost")]
    pub similarity_boost: f64,
}

/// Google Cloud TTS provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "tts.google"]
pub struct GoogleTtsConfig {
    /// API key for Google Cloud TTS.
    #[serde(default)]
    #[secret]
    pub api_key: Option<String>,
    /// Language code (default `"en-US"`).
    #[serde(default = "default_google_tts_language_code")]
    pub language_code: String,
}

/// Edge TTS provider configuration (free, subprocess-based).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "tts.edge"]
pub struct EdgeTtsConfig {
    /// Path to the `edge-tts` binary (default `"edge-tts"`).
    #[serde(default = "default_edge_tts_binary_path")]
    pub binary_path: String,
}

impl Default for EdgeTtsConfig {
    fn default() -> Self {
        Self {
            binary_path: default_edge_tts_binary_path(),
        }
    }
}

/// Piper TTS provider configuration (local GPU-accelerated, OpenAI-compatible endpoint).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "tts.piper"]
pub struct PiperTtsConfig {
    /// Base URL for the Piper TTS HTTP server (e.g. `"http://127.0.0.1:5000/v1/audio/speech"`).
    #[serde(default = "default_piper_tts_api_url")]
    pub api_url: String,
}

impl Default for PiperTtsConfig {
    fn default() -> Self {
        Self {
            api_url: default_piper_tts_api_url(),
        }
    }
}
