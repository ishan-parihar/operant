//! Text-to-Speech Provider Trait
//!
//! Defines the pluggable-backend interface for TTS synthesis.
//! Providers register with `TtsPluginRegistry`; the active one
//! services `text_to_speech` tool calls when the configured name
//! is neither a built-in nor a command-type provider.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Output audio format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum AudioFormat {
    #[default]
    Mp3,
    Wav,
    Ogg,
    Opus,
    Flac,
}

impl fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mp3 => write!(f, "mp3"),
            Self::Wav => write!(f, "wav"),
            Self::Ogg => write!(f, "ogg"),
            Self::Opus => write!(f, "opus"),
            Self::Flac => write!(f, "flac"),
        }
    }
}

/// Voice catalog entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<String>,
}

/// Model catalog entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_text_length: Option<usize>,
}

/// Result of TTS synthesis
#[derive(Debug, Clone)]
pub struct SynthesisResult {
    /// Path to the written audio file
    pub output_path: String,
    /// Audio format of the output
    pub format: AudioFormat,
    /// Whether the output is suitable for voice-bubble delivery (Telegram)
    pub voice_compatible: bool,
}

/// Abstract trait for a TTS backend.
///
/// Implement this trait to create a custom TTS provider that can be
/// registered with `TtsPluginRegistry`.
#[async_trait]
pub trait TtsProvider: Send + Sync {
    /// Stable short identifier (lowercase, no spaces).
    /// Names colliding with built-ins are rejected at registration.
    fn name(&self) -> &str;

    /// Human-readable display name (defaults to name.title())
    fn display_name(&self) -> String {
        let name = self.name();
        let mut chars = name.chars();
        match chars.next() {
            Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            None => String::new(),
        }
    }

    /// Whether this provider can service calls right now.
    /// Should not raise — used for availability checks.
    fn is_available(&self) -> bool {
        true
    }

    /// Voice catalog entries. Empty if provider doesn't enumerate voices.
    fn list_voices(&self) -> Vec<VoiceInfo> {
        Vec::new()
    }

    /// Model catalog entries. Empty if provider has a single fixed model.
    fn list_models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    /// Default voice id, if applicable.
    fn default_voice(&self) -> Option<&str> {
        None
    }

    /// Default model id, if applicable.
    fn default_model(&self) -> Option<&str> {
        None
    }

    /// Whether output is suitable for voice-bubble delivery (Telegram).
    /// When true, the dispatcher runs Opus conversion if needed.
    fn voice_compatible(&self) -> bool {
        false
    }

    /// Synthesize text and write audio to output_path.
    /// Returns the path to the written file.
    /// Raises on failure — the dispatcher converts to error envelope.
    async fn synthesize(
        &self,
        text: &str,
        output_path: &str,
        voice: Option<&str>,
        model: Option<&str>,
        format: AudioFormat,
    ) -> Result<SynthesisResult, TtsError>;
}

/// Error type for TTS operations
#[derive(Debug, thiserror::Error)]
pub enum TtsError {
    #[error("TTS provider '{provider}' not available: {reason}")]
    ProviderUnavailable { provider: String, reason: String },

    #[error("TTS synthesis failed: {0}")]
    SynthesisFailed(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Provider timed out after {timeout_secs}s")]
    Timeout { timeout_secs: u64 },

    #[error("{0}")]
    Other(String),
}

/// Parse an AudioFormat from a string extension or name
pub fn parse_audio_format(s: &str) -> AudioFormat {
    match s.to_lowercase().as_str() {
        "mp3" => AudioFormat::Mp3,
        "wav" => AudioFormat::Wav,
        "ogg" | "opus" => AudioFormat::Ogg,
        "flac" => AudioFormat::Flac,
        _ => AudioFormat::Mp3,
    }
}
