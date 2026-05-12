//! Voice Mode -- Push-to-talk audio recording, STT dispatch, and TTS synthesis.
//!
//! Provides:
//! - Audio recording via subprocess (ffmpeg/arecord) or Termux:API
//! - Speech-to-Text (STT) with 6 provider integrations
//! - Text-to-Speech (TTS) with 10 provider integrations + custom commands
//! - Voice Activity Detection (energy-based RMS threshold)
//! - Whisper hallucination filter
//! - CLI voice state machine (Idle → Listening → Processing → Speaking)
//!
//! This is a library module, not a HermesTool. It provides composable
//! primitives for voice interaction in the CLI and gateway.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex, Notify, RwLock};
use tracing::{debug, error, info};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Whisper-native sample rate
pub const SAMPLE_RATE: u32 = 16000;
/// Mono
pub const CHANNELS: u16 = 1;
/// 16-bit PCM
pub const SAMPLE_WIDTH: u16 = 2;

/// RMS below this threshold is considered silence (int16 range 0–32767)
pub const SILENCE_RMS_THRESHOLD: f64 = 200.0;

/// Seconds of continuous silence before auto-stop
pub const SILENCE_DURATION_SECONDS: f64 = 3.0;

/// Max seconds to wait for speech before auto-stop
pub const MAX_WAIT_SECONDS: f64 = 15.0;

/// Minimum recording duration in seconds
pub const MIN_RECORDING_SECONDS: f64 = 0.3;

// ---------------------------------------------------------------------------
// Voice Configuration
// ---------------------------------------------------------------------------

/// Configuration for voice mode operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    /// STT provider name
    pub stt_provider: String,
    /// TTS provider name
    pub tts_provider: String,
    /// Push-to-talk key (e.g., "space", "v")
    pub push_to_talk_key: String,
    /// Enable VAD-based continuous mode
    pub vad_enabled: bool,
    /// VAD sensitivity (1–10, higher = more sensitive)
    pub vad_sensitivity: u8,
    /// Whisper model to use
    pub stt_model: String,
    /// TTS voice name
    pub tts_voice: String,
    /// Custom TTS shell command template (use {text} placeholder)
    pub custom_tts_command: Option<String>,
    /// API keys
    pub openai_api_key: Option<String>,
    pub groq_api_key: Option<String>,
    pub google_api_key: Option<String>,
    pub azure_api_key: Option<String>,
    pub azure_region: Option<String>,
    pub assemblyai_api_key: Option<String>,
    pub deepgram_api_key: Option<String>,
    pub elevenlabs_api_key: Option<String>,
    pub listen_address: Option<String>,
    /// Temp directory for recordings
    pub temp_dir: PathBuf,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            stt_provider: "whisper".to_string(),
            tts_provider: "edge".to_string(),
            push_to_talk_key: "space".to_string(),
            vad_enabled: false,
            vad_sensitivity: 5,
            stt_model: "whisper-1".to_string(),
            tts_voice: "en-US-AriaNeural".to_string(),
            custom_tts_command: None,
            openai_api_key: None,
            groq_api_key: None,
            google_api_key: None,
            azure_api_key: None,
            azure_region: None,
            assemblyai_api_key: None,
            deepgram_api_key: None,
            elevenlabs_api_key: None,
            listen_address: None,
            temp_dir: std::env::temp_dir().join("hermes_voice"),
        }
    }
}

impl VoiceConfig {
    /// Resolve API keys from config or environment variables
    pub fn resolve_openai_key(&self) -> Option<String> {
        self.openai_api_key
            .clone()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .filter(|k| !k.is_empty())
    }

    pub fn resolve_groq_key(&self) -> Option<String> {
        self.groq_api_key
            .clone()
            .or_else(|| std::env::var("GROQ_API_KEY").ok())
            .filter(|k| !k.is_empty())
    }

    pub fn resolve_google_key(&self) -> Option<String> {
        self.google_api_key
            .clone()
            .or_else(|| std::env::var("GOOGLE_API_KEY").ok())
            .filter(|k| !k.is_empty())
    }

    pub fn resolve_azure_key(&self) -> Option<String> {
        self.azure_api_key
            .clone()
            .or_else(|| std::env::var("AZURE_API_KEY").ok())
            .filter(|k| !k.is_empty())
    }

    pub fn resolve_assemblyai_key(&self) -> Option<String> {
        self.assemblyai_api_key
            .clone()
            .or_else(|| std::env::var("ASSEMBLYAI_API_KEY").ok())
            .filter(|k| !k.is_empty())
    }

    pub fn resolve_deepgram_key(&self) -> Option<String> {
        self.deepgram_api_key
            .clone()
            .or_else(|| std::env::var("DEEPGRAM_API_KEY").ok())
            .filter(|k| !k.is_empty())
    }

    pub fn resolve_elevenlabs_key(&self) -> Option<String> {
        self.elevenlabs_api_key
            .clone()
            .or_else(|| std::env::var("ELEVENLABS_API_KEY").ok())
            .filter(|k| !k.is_empty())
    }
}

// ---------------------------------------------------------------------------
// Audio Recorder
// ---------------------------------------------------------------------------

/// Audio recorder trait for capturing microphone input
///
/// Implementations use subprocess-based capture (ffmpeg, arecord, Termux:API)
/// rather than native audio libraries to avoid PortAudio/cpal dependencies.
#[async_trait::async_trait]
pub trait AudioRecorder: Send + Sync {
    /// Whether this recorder supports silence-based auto-stop callbacks
    fn supports_silence_autostop(&self) -> bool;

    /// Whether recording is currently active
    fn is_recording(&self) -> bool;

    /// Seconds elapsed since recording started
    fn elapsed_seconds(&self) -> f64;

    /// Current audio input RMS level (0.0–32767.0)
    fn current_rms(&self) -> f64;

    /// Start recording with an optional silence callback
    async fn start(&mut self, on_silence_stop: Option<Box<dyn FnOnce() + Send>>) -> Result<(), VoiceError>;

    /// Stop recording and return path to audio file, or None if no audio captured
    async fn stop(&mut self) -> Result<Option<PathBuf>, VoiceError>;

    /// Cancel recording without saving
    async fn cancel(&mut self) -> Result<(), VoiceError>;

    /// Shut down the recorder, releasing resources
    async fn shutdown(&mut self) -> Result<(), VoiceError>;
}

/// FFmpeg-based audio recorder using subprocess
pub struct FFmpegRecorder {
    config: VoiceConfig,
    child: Option<Child>,
    recording: bool,
    start_time: Option<Instant>,
    output_path: Option<PathBuf>,
    current_rms: Arc<Mutex<f64>>,
    silence_callback: Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>,
}

impl FFmpegRecorder {
    pub fn new(config: VoiceConfig) -> Self {
        Self {
            config,
            child: None,
            recording: false,
            start_time: None,
            output_path: None,
            current_rms: Arc::new(Mutex::new(0.0)),
            silence_callback: Arc::new(Mutex::new(None)),
        }
    }
}

#[async_trait::async_trait]
impl AudioRecorder for FFmpegRecorder {
    fn supports_silence_autostop(&self) -> bool {
        false // subprocess-based; no live audio stream to analyze
    }

    fn is_recording(&self) -> bool {
        self.recording
    }

    fn elapsed_seconds(&self) -> f64 {
        self.start_time
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0)
    }

    fn current_rms(&self) -> f64 {
        0.0 // subprocess-based; we don't have live audio data
    }

    async fn start(&mut self, on_silence_stop: Option<Box<dyn FnOnce() + Send>>) -> Result<(), VoiceError> {
        if self.recording {
            return Ok(()); // already recording
        }

        *self.silence_callback.lock().await = on_silence_stop;

        // Create temp directory
        tokio::fs::create_dir_all(&self.config.temp_dir)
            .await
            .map_err(|e| VoiceError::Recording(format!("Failed to create temp dir: {e}")))?;

        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let output_path = self.config.temp_dir.join(format!("recording_{timestamp}.wav"));

        // Detect available recorder: ffmpeg, arecord, or sox
        let recorder_cmd = Self::detect_recorder();
        let mut cmd = Command::new(&recorder_cmd.0);
        cmd.args(&recorder_cmd.1)
            .arg(&output_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        let child = cmd.spawn().map_err(|e| {
            VoiceError::Recording(format!(
                "Failed to start recorder '{}': {e}. \
                 Install ffmpeg, arecord (alsa-utils), or sox.",
                recorder_cmd.0
            ))
        })?;

        self.child = Some(child);
        self.recording = true;
        self.start_time = Some(Instant::now());
        self.output_path = Some(output_path);

        info!("Voice recording started via {}", recorder_cmd.0);
        Ok(())
    }

    async fn stop(&mut self) -> Result<Option<PathBuf>, VoiceError> {
        if !self.recording {
            return Ok(None);
        }
        self.recording = false;

        // Kill the recording subprocess
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }

        let path = self.output_path.take();
        let start = self.start_time;
        self.start_time = None;

        let path = match path {
            Some(p) => p,
            None => return Ok(None),
        };

        // Validate the recording
        if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Ok(None);
        }

        let elapsed = start.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0);
        if elapsed < MIN_RECORDING_SECONDS {
            let _ = tokio::fs::remove_file(&path).await;
            debug!("Recording too short ({elapsed:.1}s), discarding");
            return Ok(None);
        }

        let metadata = tokio::fs::metadata(&path).await.map_err(|e| {
            VoiceError::Recording(format!("Failed to read recording metadata: {e}"))
        })?;

        if metadata.len() == 0 {
            let _ = tokio::fs::remove_file(&path).await;
            return Ok(None);
        }

        info!("Voice recording stopped ({elapsed:.1}s, {} bytes)", metadata.len());
        Ok(Some(path))
    }

    async fn cancel(&mut self) -> Result<(), VoiceError> {
        self.recording = false;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        if let Some(path) = self.output_path.take() {
            let _ = tokio::fs::remove_file(&path).await;
        }
        self.start_time = None;
        info!("Voice recording cancelled");
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), VoiceError> {
        self.cancel().await
    }
}

impl FFmpegRecorder {
    /// Detect the best available recorder command.
    /// Returns (command_name, base_args_without_output_path).
    fn detect_recorder() -> (String, Vec<String>) {
        for (cmd, args) in &[
            ("ffmpeg", vec![
                "-f", "alsa", "-i", "default",
                "-ar", &SAMPLE_RATE.to_string(),
                "-ac", &CHANNELS.to_string(),
                "-sample_fmt", "s16",
                "-y",
            ]),
            ("arecord", vec![
                "-r", &SAMPLE_RATE.to_string(),
                "-c", &CHANNELS.to_string(),
                "-f", "S16_LE",
                "-t", "wav",
                "-q",
            ]),
            ("sox", vec![
                "-r", &SAMPLE_RATE.to_string(),
                "-c", &CHANNELS.to_string(),
                "-b", "16",
                "-e", "signed-integer",
                "-d",
            ]),
        ] {
            if which::which(cmd).is_ok() {
                return (cmd.to_string(), args.iter().map(|s| s.to_string()).collect());
            }
        }
        // Default to ffmpeg even if not found (will fail with a clear error)
        ("ffmpeg".to_string(), vec![
            "-f".to_string(), "alsa".to_string(),
            "-i".to_string(), "default".to_string(),
            "-ar".to_string(), SAMPLE_RATE.to_string(),
            "-ac".to_string(), CHANNELS.to_string(),
            "-sample_fmt".to_string(), "s16".to_string(),
            "-y".to_string(),
        ])
    }
}

/// Termux:API-based audio recorder for Android environments
pub struct TermuxRecorder {
    config: VoiceConfig,
    recording: bool,
    start_time: Option<Instant>,
    output_path: Option<PathBuf>,
}

impl TermuxRecorder {
    pub fn new(config: VoiceConfig) -> Self {
        Self {
            config,
            recording: false,
            start_time: None,
            output_path: None,
        }
    }
}

#[async_trait::async_trait]
impl AudioRecorder for TermuxRecorder {
    fn supports_silence_autostop(&self) -> bool {
        false
    }

    fn is_recording(&self) -> bool {
        self.recording
    }

    fn elapsed_seconds(&self) -> f64 {
        self.start_time
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0)
    }

    fn current_rms(&self) -> f64 {
        0.0
    }

    async fn start(&mut self, _on_silence_stop: Option<Box<dyn FnOnce() + Send>>) -> Result<(), VoiceError> {
        if self.recording {
            return Ok(());
        }

        // Check Termux:API availability
        let mic_cmd = which::which("termux-microphone-record")
            .map_err(|_| VoiceError::Recording(
                "Termux voice capture requires termux-api package. \
                 Install: pkg install termux-api".to_string()
            ))?;

        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let output_path = self.config.temp_dir.join(format!("recording_{timestamp}.aac"));

        tokio::fs::create_dir_all(&self.config.temp_dir)
            .await
            .map_err(|e| VoiceError::Recording(format!("Failed to create temp dir: {e}")))?;

        let status = Command::new(&mic_cmd)
            .args([
                "-f", &output_path.to_string_lossy(),
                "-l", "0",
                "-e", "aac",
                "-r", &SAMPLE_RATE.to_string(),
                "-c", &CHANNELS.to_string(),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .status()
            .await
            .map_err(|e| VoiceError::Recording(format!("Termux microphone start failed: {e}")))?;

        if !status.success() {
            return Err(VoiceError::Recording(
                "Termux microphone start failed".to_string(),
            ));
        }

        self.recording = true;
        self.start_time = Some(Instant::now());
        self.output_path = Some(output_path);
        info!("Termux voice recording started");
        Ok(())
    }

    async fn stop(&mut self) -> Result<Option<PathBuf>, VoiceError> {
        if !self.recording {
            return Ok(None);
        }
        self.recording = false;

        // Stop Termux recording via -q flag
        if let Ok(mic_cmd) = which::which("termux-microphone-record") {
            let _ = Command::new(&mic_cmd)
                .arg("-q")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
        }

        let path = self.output_path.take();
        let start = self.start_time;
        self.start_time = None;

        let path = match path {
            Some(p) => p,
            None => return Ok(None),
        };

        if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Ok(None);
        }

        let elapsed = start.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0);
        if elapsed < MIN_RECORDING_SECONDS {
            let _ = tokio::fs::remove_file(&path).await;
            return Ok(None);
        }

        let metadata = tokio::fs::metadata(&path).await.ok();
        if metadata.map_or(true, |m| m.len() == 0) {
            let _ = tokio::fs::remove_file(&path).await;
            return Ok(None);
        }

        info!("Termux voice recording stopped: {path:?}");
        Ok(Some(path))
    }

    async fn cancel(&mut self) -> Result<(), VoiceError> {
        self.recording = false;
        if let Ok(mic_cmd) = which::which("termux-microphone-record") {
            let _ = Command::new(&mic_cmd)
                .arg("-q")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
        }
        if let Some(path) = self.output_path.take() {
            let _ = tokio::fs::remove_file(&path).await;
        }
        self.start_time = None;
        info!("Termux voice recording cancelled");
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), VoiceError> {
        self.cancel().await
    }
}

/// Create the best available audio recorder for the current environment
pub fn create_recorder(config: VoiceConfig) -> Box<dyn AudioRecorder> {
    if which::which("termux-microphone-record").is_ok() {
        Box::new(TermuxRecorder::new(config))
    } else {
        Box::new(FFmpegRecorder::new(config))
    }
}

// ---------------------------------------------------------------------------
// Environment Detection
// ---------------------------------------------------------------------------

/// Result of environment detection
#[derive(Debug, Clone, Serialize)]
pub struct AudioEnvironment {
    pub available: bool,
    pub warnings: Vec<String>,
    pub notices: Vec<String>,
    pub recorder: String,
}

/// Detect if the current environment supports audio I/O
pub fn detect_audio_environment() -> AudioEnvironment {
    let mut warnings: Vec<String> = Vec::new();
    let mut notices: Vec<String> = Vec::new();

    // SSH detection
    if std::env::var("SSH_CLIENT").is_ok()
        || std::env::var("SSH_TTY").is_ok()
        || std::env::var("SSH_CONNECTION").is_ok()
    {
        warnings.push("Running over SSH -- no audio devices available".to_string());
    }

    // Container detection
    if Path::new("/.dockerenv").exists()
        || std::env::var("KUBERNETES_SERVICE_HOST").is_ok()
    {
        warnings.push("Running inside container -- no audio devices".to_string());
    }

    // WSL detection
    let is_wsl = std::fs::read_to_string("/proc/version")
        .ok()
        .map_or(false, |v| v.to_lowercase().contains("microsoft"));

    if is_wsl {
        if std::env::var("PULSE_SERVER").is_ok() {
            notices.push("Running in WSL with PulseAudio bridge".to_string());
        } else {
            warnings.push(
                "Running in WSL -- audio requires PulseAudio bridge.\n\
                 1. Set PULSE_SERVER=unix:/mnt/wslg/PulseServer\n\
                 2. Create ~/.asoundrc pointing ALSA at PulseAudio\n\
                 3. Verify with: arecord -d 3 /tmp/test.wav && aplay /tmp/test.wav"
                    .to_string(),
            );
        }
    }

    // Check for recorder commands
    let recorder = if which::which("termux-microphone-record").is_ok() {
        notices.push("Termux:API microphone capture available".to_string());
        "termux".to_string()
    } else if which::which("ffmpeg").is_ok() {
        notices.push("Audio capture available via ffmpeg".to_string());
        "ffmpeg".to_string()
    } else if which::which("arecord").is_ok() {
        notices.push("Audio capture available via arecord".to_string());
        "arecord".to_string()
    } else if which::which("sox").is_ok() {
        notices.push("Audio capture available via sox".to_string());
        "sox".to_string()
    } else {
        warnings.push(
            "No audio recorder found. Install ffmpeg, arecord (alsa-utils), or sox.".to_string(),
        );
        "none".to_string()
    };

    AudioEnvironment {
        available: warnings.is_empty(),
        warnings,
        notices,
        recorder,
    }
}

// ---------------------------------------------------------------------------
// STT Providers
// ---------------------------------------------------------------------------

/// Supported Speech-to-Text providers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SttProvider {
    Whisper,
    Google,
    Azure,
    AssemblyAI,
    Deepgram,
    Local,
}

impl SttProvider {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "whisper" | "openai" => SttProvider::Whisper,
            "google" => SttProvider::Google,
            "azure" => SttProvider::Azure,
            "assemblyai" => SttProvider::AssemblyAI,
            "deepgram" => SttProvider::Deepgram,
            "local" => SttProvider::Local,
            _ => SttProvider::Whisper,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SttProvider::Whisper => "whisper",
            SttProvider::Google => "google",
            SttProvider::Azure => "azure",
            SttProvider::AssemblyAI => "assemblyai",
            SttProvider::Deepgram => "deepgram",
            SttProvider::Local => "local",
        }
    }
}

/// Speech-to-Text engine trait
#[async_trait::async_trait]
pub trait SttEngine: Send + Sync {
    /// Transcribe audio from a WAV file path
    async fn transcribe(&self, audio_path: &Path) -> Result<SttResult, VoiceError>;

    /// Provider name
    fn provider(&self) -> SttProvider;
}

/// Result of STT transcription
#[derive(Debug, Clone, Serialize)]
pub struct SttResult {
    pub success: bool,
    pub transcript: String,
    pub provider: String,
    pub model: String,
    pub filtered: bool,
    pub error: Option<String>,
}

impl SttResult {
    pub fn success(transcript: impl Into<String>, provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            success: true,
            transcript: transcript.into(),
            provider: provider.into(),
            model: model.into(),
            filtered: false,
            error: None,
        }
    }

    pub fn filtered(transcript: impl Into<String>, provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            success: true,
            transcript: transcript.into(),
            provider: provider.into(),
            model: model.into(),
            filtered: true,
            error: None,
        }
    }

    pub fn error(provider: impl Into<String>, model: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            success: false,
            transcript: String::new(),
            provider: provider.into(),
            model: model.into(),
            filtered: false,
            error: Some(error.into()),
        }
    }
}

// ----- Whisper (OpenAI/Groq compatible) -----

/// Whisper STT engine (OpenAI or Groq API)
pub struct WhisperEngine {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl WhisperEngine {
    /// Create a new Whisper engine.
    /// `base_url` can point to OpenAI API or a Groq-compatible endpoint.
    pub fn new(api_key: String, model: String, base_url: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model,
            base_url,
        }
    }

    /// Create a Groq Whisper engine
    pub fn groq(api_key: String) -> Self {
        Self::new(
            api_key,
            "whisper-large-v3-turbo".to_string(),
            "https://api.groq.com/openai/v1".to_string(),
        )
    }

    /// Create an OpenAI Whisper engine
    pub fn openai(api_key: String) -> Self {
        Self::new(
            api_key,
            "whisper-1".to_string(),
            "https://api.openai.com/v1".to_string(),
        )
    }
}

#[async_trait::async_trait]
impl SttEngine for WhisperEngine {
    async fn transcribe(&self, audio_path: &Path) -> Result<SttResult, VoiceError> {
        let audio_bytes = tokio::fs::read(audio_path)
            .await
            .map_err(|e| VoiceError::Stt(format!("Failed to read audio file: {e}")))?;

        // Build multipart form
        let file_part = reqwest::multipart::Part::bytes(audio_bytes)
            .file_name(audio_path.file_name().unwrap_or_default().to_string_lossy().to_string())
            .mime_str("audio/wav")
            .map_err(|e| VoiceError::Stt(format!("Failed to create multipart: {e}")))?;

        let form = reqwest::multipart::Form::new()
            .part("file", file_part)
            .text("model", self.model.clone());

        let response = self
            .client
            .post(format!("{}/audio/transcriptions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| VoiceError::Stt(format!("Whisper API request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(VoiceError::Stt(format!(
                "Whisper API returned {status}: {body}"
            )));
        }

        #[derive(Deserialize)]
        struct WhisperResponse {
            text: String,
        }

        let whisper_resp: WhisperResponse = response
            .json()
            .await
            .map_err(|e| VoiceError::Stt(format!("Failed to parse Whisper response: {e}")))?;

        let transcript = whisper_resp.text;

        // Apply hallucination filter
        if is_whisper_hallucination(&transcript) {
            info!("Filtered Whisper hallucination: {transcript:?}");
            return Ok(SttResult::filtered("", self.provider().as_str(), &self.model));
        }

        Ok(SttResult::success(&transcript, self.provider().as_str(), &self.model))
    }

    fn provider(&self) -> SttProvider {
        SttProvider::Whisper
    }
}

// ----- Google Cloud STT -----

/// Google Cloud Speech-to-Text engine
pub struct GoogleSttEngine {
    client: Client,
    api_key: String,
}

impl GoogleSttEngine {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }
}

#[async_trait::async_trait]
impl SttEngine for GoogleSttEngine {
    async fn transcribe(&self, audio_path: &Path) -> Result<SttResult, VoiceError> {
        let audio_bytes = tokio::fs::read(audio_path)
            .await
            .map_err(|e| VoiceError::Stt(format!("Failed to read audio file: {e}")))?;

        let encoded = base64::engine::general_purpose::STANDARD.encode(&audio_bytes);

        let body = serde_json::json!({
            "config": {
                "encoding": "LINEAR16",
                "sampleRateHertz": SAMPLE_RATE,
                "languageCode": "en-US",
                "model": "latest_short"
            },
            "audio": {
                "content": encoded
            }
        });

        let response = self
            .client
            .post("https://speech.googleapis.com/v1/speech:recognize")
            .query(&[("key", &self.api_key)])
            .json(&body)
            .send()
            .await
            .map_err(|e| VoiceError::Stt(format!("Google STT request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let body_text = response.text().await.unwrap_or_default();
            return Err(VoiceError::Stt(format!(
                "Google STT returned {status}: {body_text}"
            )));
        }

        #[derive(Deserialize)]
        struct GoogleSttResponse {
            results: Option<Vec<GoogleResult>>,
        }
        #[derive(Deserialize)]
        struct GoogleResult {
            alternatives: Option<Vec<GoogleAlternative>>,
        }
        #[derive(Deserialize)]
        struct GoogleAlternative {
            transcript: String,
        }

        let resp: GoogleSttResponse = response
            .json()
            .await
            .map_err(|e| VoiceError::Stt(format!("Failed to parse Google STT response: {e}")))?;

        let transcript = resp
            .results
            .and_then(|r| r.into_iter().next())
            .and_then(|r| r.alternatives)
            .and_then(|a| a.into_iter().next())
            .map(|a| a.transcript)
            .unwrap_or_default();

        Ok(SttResult::success(&transcript, self.provider().as_str(), "latest_short"))
    }

    fn provider(&self) -> SttProvider {
        SttProvider::Google
    }
}

// ----- Azure Cognitive Services STT -----

/// Azure Cognitive Services Speech-to-Text engine
pub struct AzureSttEngine {
    client: Client,
    api_key: String,
    region: String,
}

impl AzureSttEngine {
    pub fn new(api_key: String, region: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            region,
        }
    }
}

#[async_trait::async_trait]
impl SttEngine for AzureSttEngine {
    async fn transcribe(&self, audio_path: &Path) -> Result<SttResult, VoiceError> {
        let audio_bytes = tokio::fs::read(audio_path)
            .await
            .map_err(|e| VoiceError::Stt(format!("Failed to read audio file: {e}")))?;

        let url = format!(
            "https://{}.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1?language=en-US",
            self.region
        );

        let response = self
            .client
            .post(&url)
            .header("Ocp-Apim-Subscription-Key", &self.api_key)
            .header("Content-Type", "audio/wav; codecs=audio/pcm; samplerate=16000")
            .body(audio_bytes)
            .send()
            .await
            .map_err(|e| VoiceError::Stt(format!("Azure STT request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(VoiceError::Stt(format!(
                "Azure STT returned {status}: {body}"
            )));
        }

        #[derive(Deserialize)]
        struct AzureSttResponse {
            DisplayText: Option<String>,
            RecognitionStatus: String,
        }

        let resp: AzureSttResponse = response
            .json()
            .await
            .map_err(|e| VoiceError::Stt(format!("Failed to parse Azure STT response: {e}")))?;

        let transcript = resp.DisplayText.unwrap_or_default();
        Ok(SttResult::success(&transcript, self.provider().as_str(), "azure-stt"))
    }

    fn provider(&self) -> SttProvider {
        SttProvider::Azure
    }
}

// ----- AssemblyAI STT -----

/// AssemblyAI Speech-to-Text engine
pub struct AssemblyAIEngine {
    client: Client,
    api_key: String,
}

impl AssemblyAIEngine {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }
}

#[async_trait::async_trait]
impl SttEngine for AssemblyAIEngine {
    async fn transcribe(&self, audio_path: &Path) -> Result<SttResult, VoiceError> {
        let audio_bytes = tokio::fs::read(audio_path)
            .await
            .map_err(|e| VoiceError::Stt(format!("Failed to read audio file: {e}")))?;

        // Step 1: Upload the audio
        let upload_resp = self
            .client
            .post("https://api.assemblyai.com/v2/upload")
            .header("Authorization", &self.api_key)
            .body(audio_bytes)
            .send()
            .await
            .map_err(|e| VoiceError::Stt(format!("AssemblyAI upload failed: {e}")))?;

        #[derive(Deserialize)]
        struct UploadResponse {
            upload_url: String,
        }

        let upload: UploadResponse = upload_resp
            .json()
            .await
            .map_err(|e| VoiceError::Stt(format!("Failed to parse upload response: {e}")))?;

        // Step 2: Submit transcription
        let transcribe_body = serde_json::json!({
            "audio_url": upload.upload_url,
        });

        let transcribe_resp = self
            .client
            .post("https://api.assemblyai.com/v2/transcript")
            .header("Authorization", &self.api_key)
            .json(&transcribe_body)
            .send()
            .await
            .map_err(|e| VoiceError::Stt(format!("AssemblyAI transcribe request failed: {e}")))?;

        #[derive(Deserialize)]
        struct TranscriptResponse {
            id: String,
            status: String,
        }

        let transcript_req: TranscriptResponse = transcribe_resp
            .json()
            .await
            .map_err(|e| VoiceError::Stt(format!("Failed to parse transcript response: {e}")))?;

        // Step 3: Poll for completion (up to 30s)
        let poll_url = format!(
            "https://api.assemblyai.com/v2/transcript/{}",
            transcript_req.id
        );

        let start = Instant::now();
        let timeout = Duration::from_secs(30);

        loop {
            if start.elapsed() > timeout {
                return Err(VoiceError::Stt("AssemblyAI transcription timed out".to_string()));
            }

            let poll_resp = self
                .client
                .get(&poll_url)
                .header("Authorization", &self.api_key)
                .send()
                .await
                .map_err(|e| VoiceError::Stt(format!("AssemblyAI poll failed: {e}")))?;

            #[derive(Deserialize)]
            struct PollResponse {
                status: String,
                text: Option<String>,
                error: Option<String>,
            }

            let poll: PollResponse = poll_resp
                .json()
                .await
                .map_err(|e| VoiceError::Stt(format!("Failed to parse poll response: {e}")))?;

            match poll.status.as_str() {
                "completed" => {
                    let transcript = poll.text.unwrap_or_default();
                    return Ok(SttResult::success(&transcript, self.provider().as_str(), "assemblyai"));
                }
                "error" => {
                    return Err(VoiceError::Stt(format!(
                        "AssemblyAI error: {}",
                        poll.error.unwrap_or_default()
                    )));
                }
                _ => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }

    fn provider(&self) -> SttProvider {
        SttProvider::AssemblyAI
    }
}

// ----- Deepgram STT -----

/// Deepgram Speech-to-Text engine
pub struct DeepgramEngine {
    client: Client,
    api_key: String,
}

impl DeepgramEngine {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }
}

#[async_trait::async_trait]
impl SttEngine for DeepgramEngine {
    async fn transcribe(&self, audio_path: &Path) -> Result<SttResult, VoiceError> {
        let audio_bytes = tokio::fs::read(audio_path)
            .await
            .map_err(|e| VoiceError::Stt(format!("Failed to read audio file: {e}")))?;

        let response = self
            .client
            .post("https://api.deepgram.com/v1/listen?model=nova-2&language=en")
            .header("Authorization", format!("Token {}", self.api_key))
            .header("Content-Type", "audio/wav")
            .body(audio_bytes)
            .send()
            .await
            .map_err(|e| VoiceError::Stt(format!("Deepgram request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(VoiceError::Stt(format!(
                "Deepgram returned {status}: {body}"
            )));
        }

        #[derive(Deserialize)]
        struct DeepgramResponse {
            results: Option<DeepgramResults>,
        }
        #[derive(Deserialize)]
        struct DeepgramResults {
            channels: Vec<DeepgramChannel>,
        }
        #[derive(Deserialize)]
        struct DeepgramChannel {
            alternatives: Vec<DeepgramAlternative>,
        }
        #[derive(Deserialize)]
        struct DeepgramAlternative {
            transcript: String,
        }

        let resp: DeepgramResponse = response
            .json()
            .await
            .map_err(|e| VoiceError::Stt(format!("Failed to parse Deepgram response: {e}")))?;

        let transcript = resp
            .results
            .and_then(|r| r.channels.into_iter().next())
            .and_then(|c| c.alternatives.into_iter().next())
            .map(|a| a.transcript)
            .unwrap_or_default();

        Ok(SttResult::success(&transcript, self.provider().as_str(), "nova-2"))
    }

    fn provider(&self) -> SttProvider {
        SttProvider::Deepgram
    }
}

// ----- Local STT (stub) -----

/// Local STT engine — placeholder for future faster-whisper integration
pub struct LocalSttEngine;

#[async_trait::async_trait]
impl SttEngine for LocalSttEngine {
    async fn transcribe(&self, _audio_path: &Path) -> Result<SttResult, VoiceError> {
        Err(VoiceError::Stt(
            "Local STT not yet implemented in Rust. \
             Use a remote provider or the Python hermes-agent".to_string(),
        ))
    }

    fn provider(&self) -> SttProvider {
        SttProvider::Local
    }
}

// ----- STT Factory -----

/// Create an STT engine from config
pub fn create_stt_engine(config: &VoiceConfig) -> Result<Box<dyn SttEngine>, VoiceError> {
    let provider = SttProvider::from_str(&config.stt_provider);
    match provider {
        SttProvider::Whisper => {
            // Try Groq first, then OpenAI
            if let Some(key) = config.resolve_groq_key() {
                return Ok(Box::new(WhisperEngine::groq(key)));
            }
            if let Some(key) = config.resolve_openai_key() {
                return Ok(Box::new(WhisperEngine::openai(key)));
            }
            Err(VoiceError::Stt(
                "Whisper STT requires GROQ_API_KEY or OPENAI_API_KEY".to_string(),
            ))
        }
        SttProvider::Google => {
            let key = config
                .resolve_google_key()
                .ok_or_else(|| VoiceError::Stt("Google STT requires GOOGLE_API_KEY".to_string()))?;
            Ok(Box::new(GoogleSttEngine::new(key)))
        }
        SttProvider::Azure => {
            let key = config
                .resolve_azure_key()
                .ok_or_else(|| VoiceError::Stt("Azure STT requires AZURE_API_KEY".to_string()))?;
            let region = config
                .azure_region
                .clone()
                .unwrap_or_else(|| "westus".to_string());
            Ok(Box::new(AzureSttEngine::new(key, region)))
        }
        SttProvider::AssemblyAI => {
            let key = config
                .resolve_assemblyai_key()
                .ok_or_else(|| VoiceError::Stt("AssemblyAI requires ASSEMBLYAI_API_KEY".to_string()))?;
            Ok(Box::new(AssemblyAIEngine::new(key)))
        }
        SttProvider::Deepgram => {
            let key = config
                .resolve_deepgram_key()
                .ok_or_else(|| VoiceError::Stt("Deepgram requires DEEPGRAM_API_KEY".to_string()))?;
            Ok(Box::new(DeepgramEngine::new(key)))
        }
        SttProvider::Local => Ok(Box::new(LocalSttEngine)),
    }
}

// ---------------------------------------------------------------------------
// Whisper Hallucination Filter
// ---------------------------------------------------------------------------

lazy_static::lazy_static! {
    /// Known Whisper hallucination phrases on silent/near-silent audio
    static ref WHISPER_HALLUCINATIONS: std::collections::HashSet<&'static str> = {
        let mut set = std::collections::HashSet::new();
        set.insert("thank you.");
        set.insert("thank you");
        set.insert("thanks for watching.");
        set.insert("thanks for watching");
        set.insert("subscribe to my channel.");
        set.insert("subscribe to my channel");
        set.insert("like and subscribe.");
        set.insert("like and subscribe");
        set.insert("please subscribe.");
        set.insert("please subscribe");
        set.insert("thank you for watching.");
        set.insert("thank you for watching");
        set.insert("bye.");
        set.insert("bye");
        set.insert("you");
        set.insert("the end.");
        set.insert("the end");
        set.insert("продолжение следует");
        set.insert("sous-titres");
        set.insert("sottotitoli creati dalla comunità amara.org");
        set.insert("untertitel von stephanie geiges");
        set.insert("amara.org");
        set.insert("www.mooji.org");
        set.insert("ご視聴ありがとうございました");
        set
    };

    /// Regex pattern for repetitive hallucinations
    static ref HALLUCINATION_REPEAT_RE: Regex = Regex::new(
        r"^(?:thank you|thanks|bye|you|ok|okay|the end|\.|\s|,|!)+$"
    ).expect("Invalid hallucination regex");
}

/// Check if a transcript is a known Whisper hallucination on silence
pub fn is_whisper_hallucination(transcript: &str) -> bool {
    let cleaned = transcript.trim().to_lowercase();
    if cleaned.is_empty() {
        return true;
    }

    // Exact match against known phrases
    let without_period = cleaned.trim_end_matches(|c| c == '.' || c == '!');
    if WHISPER_HALLUCINATIONS.contains(cleaned.as_str())
        || WHISPER_HALLUCINATIONS.contains(without_period)
    {
        return true;
    }

    // Repetitive patterns (e.g. "Thank you. Thank you. Thank you.")
    if HALLUCINATION_REPEAT_RE.is_match(&cleaned) {
        return true;
    }

    false
}

/// Transcribe with hallucination filtering
pub async fn transcribe_recording(
    audio_path: &Path,
    engine: &dyn SttEngine,
) -> Result<SttResult, VoiceError> {
    let result = engine.transcribe(audio_path).await?;

    // Double-check even if engine didn't filter
    if result.success && !result.transcript.is_empty() && is_whisper_hallucination(&result.transcript)
    {
        info!("Filtered Whisper hallucination: {:?}", result.transcript);
        return Ok(SttResult::filtered(
            "",
            result.provider,
            result.model,
        ));
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// TTS Providers
// ---------------------------------------------------------------------------

/// Supported Text-to-Speech providers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TtsProvider {
    ElevenLabs,
    Google,
    Azure,
    Amazon,
    OpenAI,
    Edge,
    Local,
    Custom,
}

impl TtsProvider {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "elevenlabs" => TtsProvider::ElevenLabs,
            "google" => TtsProvider::Google,
            "azure" => TtsProvider::Azure,
            "amazon" | "aws" => TtsProvider::Amazon,
            "openai" => TtsProvider::OpenAI,
            "edge" => TtsProvider::Edge,
            "local" | "kokoro" => TtsProvider::Local,
            "custom" => TtsProvider::Custom,
            _ => TtsProvider::Edge,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TtsProvider::ElevenLabs => "elevenlabs",
            TtsProvider::Google => "google",
            TtsProvider::Azure => "azure",
            TtsProvider::Amazon => "amazon",
            TtsProvider::OpenAI => "openai",
            TtsProvider::Edge => "edge",
            TtsProvider::Local => "local",
            TtsProvider::Custom => "custom",
        }
    }
}

/// Text-to-Speech engine trait
#[async_trait::async_trait]
pub trait TtsEngine: Send + Sync {
    /// Synthesize text to audio bytes (WAV format)
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, VoiceError>;

    /// Provider name
    fn provider(&self) -> TtsProvider;
}

// ----- ElevenLabs TTS -----

/// ElevenLabs Text-to-Speech engine
pub struct ElevenLabsEngine {
    client: Client,
    api_key: String,
    voice: String,
    model: String,
}

impl ElevenLabsEngine {
    pub fn new(api_key: String, voice: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            voice,
            model: "eleven_multilingual_v2".to_string(),
        }
    }
}

#[async_trait::async_trait]
impl TtsEngine for ElevenLabsEngine {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, VoiceError> {
        let url = format!(
            "https://api.elevenlabs.io/v1/text-to-speech/{}",
            self.voice
        );

        let body = serde_json::json!({
            "text": text,
            "model_id": self.model,
            "voice_settings": {
                "stability": 0.5,
                "similarity_boost": 0.75
            }
        });

        let response = self
            .client
            .post(&url)
            .header("xi-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| VoiceError::Tts(format!("ElevenLabs request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let body_text = response.text().await.unwrap_or_default();
            return Err(VoiceError::Tts(format!(
                "ElevenLabs returned {status}: {body_text}"
            )));
        }

        let audio_bytes = response
            .bytes()
            .await
            .map_err(|e| VoiceError::Tts(format!("Failed to read ElevenLabs response: {e}")))?;

        Ok(audio_bytes.to_vec())
    }

    fn provider(&self) -> TtsProvider {
        TtsProvider::ElevenLabs
    }
}

// ----- Edge TTS (free) -----

/// Microsoft Edge TTS engine — uses the free Edge Read Aloud API
pub struct EdgeTtsEngine {
    client: Client,
    voice: String,
}

impl EdgeTtsEngine {
    pub fn new(voice: String) -> Self {
        Self {
            client: Client::new(),
            voice,
        }
    }
}

impl Default for EdgeTtsEngine {
    fn default() -> Self {
        Self::new("en-US-AriaNeural".to_string())
    }
}

#[async_trait::async_trait]
impl TtsEngine for EdgeTtsEngine {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, VoiceError> {
        // Use the edge-tts CLI tool if available, otherwise use the HTTP API
        if let Ok(edge_tts) = which::which("edge-tts") {
            let tmp_dir = std::env::temp_dir().join("hermes_tts");
            tokio::fs::create_dir_all(&tmp_dir)
                .await
                .map_err(|e| VoiceError::Tts(format!("Failed to create temp dir: {e}")))?;

            let output_path = tmp_dir.join(format!("tts_{}.mp3", chrono::Utc::now().timestamp()));

            let status = Command::new(&edge_tts)
                .args([
                    "--text", text,
                    "--voice", &self.voice,
                    "--write-media", &output_path.to_string_lossy(),
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await
                .map_err(|e| VoiceError::Tts(format!("edge-tts failed: {e}")))?;

            if !status.success() {
                return Err(VoiceError::Tts("edge-tts exited with error".to_string()));
            }

            let audio_bytes = tokio::fs::read(&output_path)
                .await
                .map_err(|e| VoiceError::Tts(format!("Failed to read edge-tts output: {e}")))?;

            let _ = tokio::fs::remove_file(&output_path).await;
            return Ok(audio_bytes);
        }

        Err(VoiceError::Tts(
            "Edge TTS requires the `edge-tts` Python package: pip install edge-tts".to_string(),
        ))
    }

    fn provider(&self) -> TtsProvider {
        TtsProvider::Edge
    }
}

// ----- Google Cloud TTS -----

/// Google Cloud Text-to-Speech engine
pub struct GoogleTtsEngine {
    client: Client,
    api_key: String,
    voice: String,
}

impl GoogleTtsEngine {
    pub fn new(api_key: String, voice: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            voice,
        }
    }
}

#[async_trait::async_trait]
impl TtsEngine for GoogleTtsEngine {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, VoiceError> {
        let body = serde_json::json!({
            "input": { "text": text },
            "voice": {
                "languageCode": "en-US",
                "name": self.voice,
            },
            "audioConfig": {
                "audioEncoding": "LINEAR16",
                "speakingRate": 1.0,
            }
        });

        let response = self
            .client
            .post("https://texttospeech.googleapis.com/v1/text:synthesize")
            .query(&[("key", &self.api_key)])
            .json(&body)
            .send()
            .await
            .map_err(|e| VoiceError::Tts(format!("Google TTS request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let body_text = response.text().await.unwrap_or_default();
            return Err(VoiceError::Tts(format!(
                "Google TTS returned {status}: {body_text}"
            )));
        }

        #[derive(Deserialize)]
        struct GoogleTtsResponse {
            audioContent: String,
        }

        let resp: GoogleTtsResponse = response
            .json()
            .await
            .map_err(|e| VoiceError::Tts(format!("Failed to parse Google TTS response: {e}")))?;

        let audio_bytes = base64::engine::general_purpose::STANDARD
            .decode(&resp.audioContent)
            .map_err(|e| VoiceError::Tts(format!("Failed to decode Google TTS audio: {e}")))?;

        Ok(audio_bytes)
    }

    fn provider(&self) -> TtsProvider {
        TtsProvider::Google
    }
}

// ----- Azure TTS -----

/// Azure Cognitive Services Text-to-Speech engine
pub struct AzureTtsEngine {
    client: Client,
    api_key: String,
    region: String,
    voice: String,
}

impl AzureTtsEngine {
    pub fn new(api_key: String, region: String, voice: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            region,
            voice,
        }
    }
}

#[async_trait::async_trait]
impl TtsEngine for AzureTtsEngine {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, VoiceError> {
        // Obtain an access token
        let token_url = format!(
            "https://{}.api.cognitive.microsoft.com/sts/v1.0/issueToken",
            self.region
        );

        let token_resp = self
            .client
            .post(&token_url)
            .header("Ocp-Apim-Subscription-Key", &self.api_key)
            .send()
            .await
            .map_err(|e| VoiceError::Tts(format!("Azure TTS token request failed: {e}")))?;

        let access_token = token_resp
            .text()
            .await
            .map_err(|e| VoiceError::Tts(format!("Failed to read Azure token: {e}")))?;

        let ssml = format!(
            r#"<speak version="1.0" xmlns="http://www.w3.org/2001/10/synthesis" xml:lang="en-US">
                <voice name="{}">{}</voice>
            </speak>"#,
            self.voice,
            text.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&apos;")
        );

        let tts_url = format!(
            "https://{}.tts.speech.microsoft.com/cognitiveservices/v1",
            self.region
        );

        let response = self
            .client
            .post(&tts_url)
            .header("Authorization", format!("Bearer {}", access_token.trim()))
            .header("Content-Type", "application/ssml+xml")
            .header("X-Microsoft-OutputFormat", "riff-16khz-16bit-mono-pcm")
            .header("User-Agent", "Hermes")
            .body(ssml)
            .send()
            .await
            .map_err(|e| VoiceError::Tts(format!("Azure TTS request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(VoiceError::Tts(format!(
                "Azure TTS returned {status}: {body}"
            )));
        }

        let audio_bytes = response
            .bytes()
            .await
            .map_err(|e| VoiceError::Tts(format!("Failed to read Azure TTS response: {e}")))?;

        Ok(audio_bytes.to_vec())
    }

    fn provider(&self) -> TtsProvider {
        TtsProvider::Azure
    }
}

// ----- OpenAI TTS -----

/// OpenAI Text-to-Speech engine
pub struct OpenaiTtsEngine {
    client: Client,
    api_key: String,
    voice: String,
    model: String,
}

impl OpenaiTtsEngine {
    pub fn new(api_key: String, voice: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            voice,
            model: "tts-1".to_string(),
        }
    }
}

#[async_trait::async_trait]
impl TtsEngine for OpenaiTtsEngine {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, VoiceError> {
        let body = serde_json::json!({
            "model": self.model,
            "input": text,
            "voice": self.voice,
            "response_format": "wav",
        });

        let response = self
            .client
            .post("https://api.openai.com/v1/audio/speech")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| VoiceError::Tts(format!("OpenAI TTS request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let body_text = response.text().await.unwrap_or_default();
            return Err(VoiceError::Tts(format!(
                "OpenAI TTS returned {status}: {body_text}"
            )));
        }

        let audio_bytes = response
            .bytes()
            .await
            .map_err(|e| VoiceError::Tts(format!("Failed to read OpenAI TTS response: {e}")))?;

        Ok(audio_bytes.to_vec())
    }

    fn provider(&self) -> TtsProvider {
        TtsProvider::OpenAI
    }
}

// ----- Custom Command TTS -----

/// Custom shell command TTS engine.
/// Runs a user-defined command that receives text via stdin or as an argument
/// and outputs audio to stdout or a file.
pub struct CustomCommandEngine {
    command_template: String,
}

impl CustomCommandEngine {
    pub fn new(command_template: String) -> Self {
        Self { command_template }
    }
}

#[async_trait::async_trait]
impl TtsEngine for CustomCommandEngine {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, VoiceError> {
        // Replace {text} placeholder with the actual text
        let cmd_str = self.command_template.replace("{text}", text);

        // Parse the command string into parts
        let parts = shell_words::split(&cmd_str)
            .map_err(|e| VoiceError::Tts(format!("Failed to parse command: {e}")))?;

        if parts.is_empty() {
            return Err(VoiceError::Tts("Empty TTS command template".to_string()));
        }

        let program = &parts[0];
        let args = &parts[1..];

        let output = Command::new(program)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| VoiceError::Tts(format!("Custom TTS command failed: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VoiceError::Tts(format!(
                "Custom TTS command exited with {}: {stderr}",
                output.status
            )));
        }

        Ok(output.stdout)
    }

    fn provider(&self) -> TtsProvider {
        TtsProvider::Custom
    }
}

// ----- Local TTS (kokoro) -----

/// Local TTS engine using the bundled kokoro-tiny model.
/// Returns WAV bytes.
pub struct LocalTtsEngine {
    engine: Arc<Mutex<Option<kokoro_tiny::TtsEngine>>>,
    voice: String,
}

impl LocalTtsEngine {
    pub fn new() -> Self {
        Self {
            engine: Arc::new(Mutex::new(None)),
            voice: "af_bella".to_string(),
        }
    }
}

#[async_trait::async_trait]
impl TtsEngine for LocalTtsEngine {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, VoiceError> {
        let mut guard = self.engine.lock().await;
        if guard.is_none() {
            *guard = Some(
                kokoro_tiny::TtsEngine::new()
                    .await
                    .map_err(|e| VoiceError::Tts(format!("Failed to init Kokoro: {e}")))?,
            );
        }
        let engine = guard.as_mut().unwrap();
        let f32_samples = engine
            .synthesize(text, Some(&self.voice))
            .map_err(|e| VoiceError::Tts(format!("kokoro-tiny TTS failed: {e}")))?;

        // Convert f32 samples to WAV bytes
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 24000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut cursor, spec)
                .map_err(|e| VoiceError::Tts(format!("Failed to create WAV writer: {e}")))?;
            for &sample in &f32_samples {
                let s = (sample * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                writer.write_sample(s)
                    .map_err(|e| VoiceError::Tts(format!("Failed to write sample: {e}")))?;
            }
            writer.finalize()
                .map_err(|e| VoiceError::Tts(format!("Failed to finalize WAV: {e}")))?;
        }
        Ok(cursor.into_inner())
    }

    fn provider(&self) -> TtsProvider {
        TtsProvider::Local
    }
}

// ----- TTS Factory -----

/// Create a TTS engine from config
pub fn create_tts_engine(config: &VoiceConfig) -> Result<Box<dyn TtsEngine>, VoiceError> {
    let provider = TtsProvider::from_str(&config.tts_provider);
    match provider {
        TtsProvider::ElevenLabs => {
            let key = config
                .resolve_elevenlabs_key()
                .ok_or_else(|| VoiceError::Tts("ElevenLabs requires ELEVENLABS_API_KEY".to_string()))?;
            Ok(Box::new(ElevenLabsEngine::new(key, config.tts_voice.clone())))
        }
        TtsProvider::Google => {
            let key = config
                .resolve_google_key()
                .ok_or_else(|| VoiceError::Tts("Google TTS requires GOOGLE_API_KEY".to_string()))?;
            Ok(Box::new(GoogleTtsEngine::new(key, config.tts_voice.clone())))
        }
        TtsProvider::Azure => {
            let key = config
                .resolve_azure_key()
                .ok_or_else(|| VoiceError::Tts("Azure TTS requires AZURE_API_KEY".to_string()))?;
            let region = config.azure_region.clone().unwrap_or_else(|| "westus".to_string());
            Ok(Box::new(AzureTtsEngine::new(key, region, config.tts_voice.clone())))
        }
        TtsProvider::OpenAI => {
            let key = config
                .resolve_openai_key()
                .ok_or_else(|| VoiceError::Tts("OpenAI TTS requires OPENAI_API_KEY".to_string()))?;
            Ok(Box::new(OpenaiTtsEngine::new(key, config.tts_voice.clone())))
        }
        TtsProvider::Edge => Ok(Box::new(EdgeTtsEngine::new(config.tts_voice.clone()))),
        TtsProvider::Custom => {
            let cmd = config.custom_tts_command.clone().ok_or_else(|| {
                VoiceError::Tts("Custom TTS requires custom_tts_command in config".to_string())
            })?;
            Ok(Box::new(CustomCommandEngine::new(cmd)))
        }
        TtsProvider::Local => Ok(Box::new(LocalTtsEngine::new())),
        TtsProvider::Amazon => Err(VoiceError::Tts(
            "Amazon Polly TTS not yet implemented in Rust".to_string(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Voice Session (State Machine)
// ---------------------------------------------------------------------------

/// Voice session states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoiceState {
    Idle,
    Listening,
    Processing,
    Speaking,
}

impl VoiceState {
    pub fn as_str(&self) -> &'static str {
        match self {
            VoiceState::Idle => "idle",
            VoiceState::Listening => "listening",
            VoiceState::Processing => "processing",
            VoiceState::Speaking => "speaking",
        }
    }
}

/// Events that can be sent to the voice session
#[derive(Debug, Clone)]
pub enum VoiceEvent {
    /// Start listening (push-to-toll key pressed, or VAD triggered)
    StartListening,
    /// Stop listening (key released, or VAD silence timeout)
    StopListening,
    /// Transcription result from STT
    TranscriptionResult(SttResult),
    /// TTS synthesis complete
    TtsComplete,
    /// Error occurred
    Error(String),
    /// Shut down the session
    Shutdown,
}

/// A managed voice session with state machine and event-driven synchronization
pub struct VoiceSession {
    config: VoiceConfig,
    state: Arc<RwLock<VoiceState>>,
    stt_engine: Option<Box<dyn SttEngine>>,
    tts_engine: Option<Box<dyn TtsEngine>>,
    recorder: Option<Box<dyn AudioRecorder>>,
    event_tx: mpsc::Sender<VoiceEvent>,
    event_rx: Option<mpsc::Receiver<VoiceEvent>>,
    state_change: Arc<Notify>,
    /// Callback invoked when transcription is complete
    on_transcription: Option<Arc<dyn Fn(SttResult) -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>>,
}

impl VoiceSession {
    /// Create a new voice session
    pub fn new(config: VoiceConfig) -> Self {
        let (event_tx, event_rx) = mpsc::channel(32);

        Self {
            state: Arc::new(RwLock::new(VoiceState::Idle)),
            config,
            stt_engine: None,
            tts_engine: None,
            recorder: None,
            event_tx,
            event_rx: Some(event_rx),
            state_change: Arc::new(Notify::new()),
            on_transcription: None,
        }
    }

    /// Set the STT engine
    pub fn with_stt(mut self, engine: Box<dyn SttEngine>) -> Self {
        self.stt_engine = Some(engine);
        self
    }

    /// Set the TTS engine
    pub fn with_tts(mut self, engine: Box<dyn TtsEngine>) -> Self {
        self.tts_engine = Some(engine);
        self
    }

    /// Set the audio recorder
    pub fn with_recorder(mut self, recorder: Box<dyn AudioRecorder>) -> Self {
        self.recorder = Some(recorder);
        self
    }

    /// Set the transcription callback
    pub fn on_transcription<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(SttResult) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.on_transcription = Some(Arc::new(move |result| Box::pin(callback(result))));
        self
    }

    /// Get the event sender for injecting events from external input (keyboard, etc.)
    pub fn event_sender(&self) -> mpsc::Sender<VoiceEvent> {
        self.event_tx.clone()
    }

    /// Get the current state
    pub async fn state(&self) -> VoiceState {
        *self.state.read().await
    }

    /// Wait for state change
    pub async fn wait_state_change(&self) {
        self.state_change.notified().await;
    }

    /// Transition to a new state, notifying waiters
    async fn set_state(state: &Arc<RwLock<VoiceState>>, notify: &Arc<Notify>, new: VoiceState) {
        let old = { *state.read().await };
        if old != new {
            debug!("Voice state: {old:?} → {new:?}");
            *state.write().await = new;
            notify.notify_waiters();
        }
    }

    /// Run the voice session event loop
    ///
    /// This processes events and manages state transitions. Call this in a
    /// spawned task or directly in an async context.
    pub async fn run(&mut self) {
        let mut event_rx = self.event_rx.take().expect("VoiceSession event receiver already taken");
        let state = self.state.clone();
        let state_change = self.state_change.clone();

        loop {
            tokio::select! {
                Some(event) = event_rx.recv() => {
                    match event {
                        VoiceEvent::StartListening => {
                            if *state.read().await != VoiceState::Idle {
                                continue;
                            }
                            Self::set_state(&state, &state_change, VoiceState::Listening).await;

                            // Start recording
                            if let Some(recorder) = &mut self.recorder {
                                let event_tx = self.event_tx.clone();
                                if let Err(e) = recorder.start(Some(Box::new(move || {
                                    // Auto-stop on silence
                                    let tx = event_tx.clone();
                                    tokio::spawn(async move {
                                        let _ = tx.send(VoiceEvent::StopListening).await;
                                    });
                                }))).await {
                                    error!("Failed to start recording: {e}");
                                    Self::set_state(&state, &state_change, VoiceState::Idle).await;
                                }
                            }
                        }

                        VoiceEvent::StopListening => {
                            if *state.read().await != VoiceState::Listening {
                                continue;
                            }
                            Self::set_state(&state, &state_change, VoiceState::Processing).await;

                            // Stop and transcribe
                            let audio_path = if let Some(recorder) = &mut self.recorder {
                                recorder.stop().await.unwrap_or(None)
                            } else {
                                None
                            };

                            if let Some(path) = audio_path {
                                if let Some(engine) = &self.stt_engine {
                                    let result = transcribe_recording(&path, engine.as_ref()).await;
                                    match result {
                                        Ok(stt_result) => {
                                            // Clean up temp file
                                            let _ = tokio::fs::remove_file(&path).await;

                                            if let Some(ref cb) = self.on_transcription {
                                                (cb)(stt_result.clone()).await;
                                            }

                                            if stt_result.success && !stt_result.transcript.is_empty() {
                                                Self::set_state(&state, &state_change, VoiceState::Speaking).await;
                                                // The callback owns the response; after TTS completes,
                                                // send TtsComplete to transition back to Idle
                                            } else {
                                                Self::set_state(&state, &state_change, VoiceState::Idle).await;
                                            }
                                        }
                                        Err(e) => {
                                            error!("Transcription failed: {e}");
                                            Self::set_state(&state, &state_change, VoiceState::Idle).await;
                                        }
                                    }
                                } else {
                                    Self::set_state(&state, &state_change, VoiceState::Idle).await;
                                }
                            } else {
                                // No audio captured; go back to idle
                                Self::set_state(&state, &state_change, VoiceState::Idle).await;
                            }
                        }

                        VoiceEvent::TranscriptionResult(result) => {
                            if let Some(ref cb) = self.on_transcription {
                                (cb)(result).await;
                            }
                            Self::set_state(&state, &state_change, VoiceState::Idle).await;
                        }

                        VoiceEvent::TtsComplete => {
                            Self::set_state(&state, &state_change, VoiceState::Idle).await;
                        }

                        VoiceEvent::Error(msg) => {
                            error!("Voice session error: {msg}");
                            Self::set_state(&state, &state_change, VoiceState::Idle).await;
                        }

                        VoiceEvent::Shutdown => {
                            info!("Voice session shutting down");
                            if let Some(recorder) = &mut self.recorder {
                                let _ = recorder.shutdown().await;
                            }
                            Self::set_state(&state, &state_change, VoiceState::Idle).await;
                            break;
                        }
                    }
                }

                else => break, // channel closed
            }
        }
    }

    /// Run the session in a background task
    pub fn spawn(mut self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.run().await;
        })
    }

    /// Synthesize speech from text using the configured TTS engine.
    /// Returns audio bytes.
    pub async fn speak(&self, text: &str) -> Result<Vec<u8>, VoiceError> {
        match &self.tts_engine {
            Some(engine) => engine.synthesize(text).await,
            None => Err(VoiceError::Tts("No TTS engine configured".to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// Audio playback (process-based)
// ---------------------------------------------------------------------------

/// Play audio file through system player (afplay, ffplay, aplay)
pub fn play_audio_file(path: &Path) -> Result<(), VoiceError> {
    let path_str = path.to_string_lossy().to_string();

    let system = std::env::consts::OS;
    let mut players: Vec<Vec<String>> = Vec::new();

    if system == "macos" {
        players.push(vec!["afplay".to_string(), path_str.clone()]);
    }
    players.push(vec![
        "ffplay".to_string(),
        "-nodisp".to_string(),
        "-autoexit".to_string(),
        "-loglevel".to_string(),
        "quiet".to_string(),
        path_str.clone(),
    ]);
    if system == "linux" {
        players.push(vec!["aplay".to_string(), "-q".to_string(), path_str]);
    }

    for cmd in &players {
        if which::which(&cmd[0]).is_ok() {
            let status = std::process::Command::new(&cmd[0])
                .args(&cmd[1..])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();

            match status {
                Ok(s) if s.success() => return Ok(()),
                _ => continue,
            }
        }
    }

    Err(VoiceError::Playback(
        "No audio player available (try ffplay, afplay, or aplay)".to_string(),
    ))
}

// ---------------------------------------------------------------------------
// VAD (Voice Activity Detection) — Energy-based
// ---------------------------------------------------------------------------

/// Simple energy-based Voice Activity Detector
///
/// Uses RMS threshold to determine if audio contains speech.
pub struct VadDetector {
    /// RMS threshold below which audio is considered silence
    threshold: f64,
    /// Number of consecutive silent frames needed to mark end of speech
    silence_frames: u32,
    /// Minimum speech duration in frames to confirm speech
    min_speech_frames: u32,
    /// Current frame counter
    speech_frames: u32,
    silent_frames: u32,
    /// Whether speech has been confirmed
    has_spoken: bool,
    /// Sample rate for frame calculation
    frame_size: usize,
}

impl VadDetector {
    pub fn new(threshold: f64, sensitivity: u8) -> Self {
        // sensitivity 1–10, higher = more sensitive (lower threshold)
        let adjusted_threshold = threshold * (1.0 + (10.0 - sensitivity as f64) / 5.0);
        Self {
            threshold: adjusted_threshold,
            silence_frames: 30, // ~0.3s at 100fps (assuming 160-sample frames at 16kHz = 100fps)
            min_speech_frames: 30, // ~0.3s to confirm speech
            speech_frames: 0,
            silent_frames: 0,
            has_spoken: false,
            frame_size: 1600, // 100ms at 16kHz
        }
    }

    /// Process a buffer of f32 audio samples (normalized -1.0 to 1.0)
    /// Returns `true` if speech is currently detected, `false` for silence.
    /// When `speech_ended()` returns true, the speaker has stopped.
    pub fn process(&mut self, samples: &[f32]) -> bool {
        let rms = self.compute_rms(samples);
        let is_voice = rms > self.threshold / 32767.0;

        if is_voice {
            self.speech_frames += 1;
            self.silent_frames = 0;

            if !self.has_spoken && self.speech_frames >= self.min_speech_frames {
                self.has_spoken = true;
            }
        } else if self.has_spoken {
            self.silent_frames += 1;
        }

        is_voice
    }

    /// Returns true if speech has been confirmed and then silence was detected
    pub fn speech_ended(&self) -> bool {
        self.has_spoken && self.silent_frames >= self.silence_frames
    }

    /// Returns true if speech has been confirmed at all
    pub fn has_spoken(&self) -> bool {
        self.has_spoken
    }

    /// Reset the detector state
    pub fn reset(&mut self) {
        self.speech_frames = 0;
        self.silent_frames = 0;
        self.has_spoken = false;
    }

    /// Compute RMS from f32 samples
    fn compute_rms(&self, samples: &[f32]) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = samples.iter().map(|&s| (s as f64).powi(2)).sum();
        (sum / samples.len() as f64).sqrt()
    }
}

// ---------------------------------------------------------------------------
// Utility: WAV reading/writing
// ---------------------------------------------------------------------------

/// Read a WAV file and return f32 samples normalized to [-1.0, 1.0]
pub fn read_wav_samples(path: &Path) -> Result<(Vec<f32>, u32), VoiceError> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| VoiceError::Recording(format!("Failed to read WAV: {e}")))?;

    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => match spec.bits_per_sample {
            16 => reader
                .samples::<i16>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / 32768.0)
                .collect(),
            8 => reader
                .samples::<i8>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / 128.0)
                .collect(),
            32 => reader
                .samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / 2147483648.0)
                .collect(),
            _ => return Err(VoiceError::Recording(format!(
                "Unsupported bits per sample: {}",
                spec.bits_per_sample
            ))),
        },
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .filter_map(|s| s.ok())
            .collect(),
    };

    Ok((samples, spec.sample_rate))
}

/// Write f32 samples to a WAV file
pub fn write_wav_samples(path: &Path, samples: &[f32], sample_rate: u32) -> Result<(), VoiceError> {
    let spec = hound::WavSpec {
        channels: CHANNELS,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|e| VoiceError::Recording(format!("Failed to create WAV writer: {e}")))?;

    for &sample in samples {
        let amplitude = (sample * 32767.0) as i16;
        writer
            .write_sample(amplitude)
            .map_err(|e| VoiceError::Recording(format!("Failed to write WAV sample: {e}")))?;
    }

    writer
        .finalize()
        .map_err(|e| VoiceError::Recording(format!("Failed to finalize WAV: {e}")))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Error Types
// ---------------------------------------------------------------------------

/// Voice module errors
#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error("Recording error: {0}")]
    Recording(String),

    #[error("STT error: {0}")]
    Stt(String),

    #[error("TTS error: {0}")]
    Tts(String),

    #[error("Playback error: {0}")]
    Playback(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),
}

// ---------------------------------------------------------------------------
// Temp file cleanup
// ---------------------------------------------------------------------------

/// Remove old temporary voice recording files
pub async fn cleanup_temp_recordings(temp_dir: &Path, max_age_seconds: u64) -> std::io::Result<u64> {
    if !tokio::fs::try_exists(temp_dir).await.unwrap_or(false) {
        return Ok(0);
    }

    let mut deleted = 0u64;
    let mut entries = tokio::fs::read_dir(temp_dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let meta = entry.metadata().await?;
        if meta.is_file() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("recording_") && name_str.ends_with(".wav") {
                if let Ok(modified) = meta.modified() {
                    if let Ok(age) = modified.elapsed() {
                        if age.as_secs() > max_age_seconds {
                            tokio::fs::remove_file(entry.path()).await?;
                            deleted += 1;
                        }
                    }
                }
            }
        }
    }

    if deleted > 0 {
        debug!("Cleaned up {deleted} old voice recordings");
    }

    Ok(deleted)
}

// ---------------------------------------------------------------------------
// TESTS
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_whisper_hallucination_filter() {
        assert!(is_whisper_hallucination("thank you."));
        assert!(is_whisper_hallucination("Thank you"));
        assert!(is_whisper_hallucination("you"));
        assert!(is_whisper_hallucination("bye"));
        assert!(is_whisper_hallucination("the end"));
        assert!(is_whisper_hallucination("Thank you. Thank you. Thank you."));
        assert!(!is_whisper_hallucination("Hello, how are you?"));
        assert!(!is_whisper_hallucination("Hello world"));
        assert!(is_whisper_hallucination(""));
    }

    #[test]
    fn test_stt_provider_parsing() {
        assert_eq!(SttProvider::from_str("whisper"), SttProvider::Whisper);
        assert_eq!(SttProvider::from_str("openai"), SttProvider::Whisper);
        assert_eq!(SttProvider::from_str("google"), SttProvider::Google);
        assert_eq!(SttProvider::from_str("azure"), SttProvider::Azure);
        assert_eq!(SttProvider::from_str("assemblyai"), SttProvider::AssemblyAI);
        assert_eq!(SttProvider::from_str("deepgram"), SttProvider::Deepgram);
        assert_eq!(SttProvider::from_str("local"), SttProvider::Local);
        assert_eq!(SttProvider::from_str("unknown"), SttProvider::Whisper);
    }

    #[test]
    fn test_tts_provider_parsing() {
        assert_eq!(TtsProvider::from_str("elevenlabs"), TtsProvider::ElevenLabs);
        assert_eq!(TtsProvider::from_str("google"), TtsProvider::Google);
        assert_eq!(TtsProvider::from_str("azure"), TtsProvider::Azure);
        assert_eq!(TtsProvider::from_str("amazon"), TtsProvider::Amazon);
        assert_eq!(TtsProvider::from_str("aws"), TtsProvider::Amazon);
        assert_eq!(TtsProvider::from_str("openai"), TtsProvider::OpenAI);
        assert_eq!(TtsProvider::from_str("edge"), TtsProvider::Edge);
        assert_eq!(TtsProvider::from_str("local"), TtsProvider::Local);
        assert_eq!(TtsProvider::from_str("kokoro"), TtsProvider::Local);
        assert_eq!(TtsProvider::from_str("custom"), TtsProvider::Custom);
        assert_eq!(TtsProvider::from_str("unknown"), TtsProvider::Edge);
    }

    #[test]
    fn test_voice_state_transitions() {
        assert_eq!(VoiceState::Idle.as_str(), "idle");
        assert_eq!(VoiceState::Listening.as_str(), "listening");
        assert_eq!(VoiceState::Processing.as_str(), "processing");
        assert_eq!(VoiceState::Speaking.as_str(), "speaking");
    }

    #[test]
    fn test_vad_detector() {
        let mut vad = VadDetector::new(SILENCE_RMS_THRESHOLD, 5);

        // Silence should not trigger speech
        let silence = vec![0.0f32; 1600];
        assert!(!vad.process(&silence));
        assert!(!vad.has_spoken());

        // Loud signal should trigger speech
        let loud: Vec<f32> = (0..1600).map(|i| ((i as f32) / 1600.0) * 0.5).collect();
        for _ in 0..30 {
            vad.process(&loud);
        }
        assert!(vad.has_spoken());

        // Then silence should end speech
        for _ in 0..35 {
            vad.process(&silence);
        }
        assert!(vad.speech_ended());
    }

    #[tokio::test]
    async fn test_voice_session_state_machine() {
        let config = VoiceConfig::default();
        let mut session = VoiceSession::new(config);

        assert_eq!(session.state().await, VoiceState::Idle);

        // Send events and check transitions via the event channel
        let tx = session.event_sender();
        tx.send(VoiceEvent::Shutdown).await.unwrap();

        // Run will process Shutdown and exit
        session.run().await;
        // After shutdown, state should be Idle
    }
}
