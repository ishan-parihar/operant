//! Voice recording + transcription bridge for the TUI.
//!
//! Backed by `operant_core::voice` — uses the real `AudioRecorder`
//! (FFmpeg or Termux subprocess) and `SttEngine` (Whisper via Groq/OpenAI,
//! Google, Azure, AssemblyAI, Deepgram, or local). API keys are resolved
//! from the `VoiceConfig` / environment variables.
//!
//! In headless environments (no microphone, no ffmpeg, no API keys),
//! recording/transcription will fail gracefully and surface an error
//! event to the TUI.

use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub enum VoiceEvent {
    RecordingStarted,
    RecordingStopped,
    Transcription(String),
    TranscriptReady(String),
    Error(String),
}

/// TUI-facing voice recorder that wraps operant-core's AudioRecorder + SttEngine.
///
/// The TUI calls `start_recording(tx)` on push-to-talk press and
/// `stop_recording()` on release. On stop, the recorded audio file is
/// transcribed via the configured STT engine and a `Transcription` event
/// is sent to `tx`.
pub struct VoiceRecorder {
    enabled: bool,
    config: operant_core::voice::VoiceConfig,
    /// Lazily-created on first start_recording; reused across sessions.
    recorder: Option<Box<dyn operant_core::voice::AudioRecorder>>,
    /// Lazily-created on first stop_recording; reused across sessions.
    stt_engine: Option<Box<dyn operant_core::voice::SttEngine>>,
    /// Event channel provided by the most recent start_recording call.
    /// Used by stop_recording to send the Transcription event.
    event_tx: Option<tokio::sync::mpsc::Sender<VoiceEvent>>,
}

impl VoiceRecorder {
    pub fn new() -> Self {
        Self {
            enabled: false,
            config: operant_core::voice::VoiceConfig::default(),
            recorder: None,
            stt_engine: None,
            event_tx: None,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check whether voice recording is available on this system.
    /// Used by `operant tui voice` to surface availability without
    /// entering the TUI.
    pub fn is_available(&self) -> bool {
        // We probe for at least one of the common recorder backends on
        // PATH. The actual recorder is created lazily on start_recording,
        // so this is a best-effort availability check.
        for cmd in &["arecord", "rec", "ffmpeg"] {
            if std::process::Command::new(cmd)
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok()
            {
                return true;
            }
        }
        false
    }

    #[expect(
        clippy::unwrap_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    /// Start recording audio. Sends `RecordingStarted` to `tx` on success.
    ///
    /// On the first call, this lazily creates the underlying AudioRecorder
    /// via `operant_core::voice::create_recorder`. If audio capture is
    /// unavailable (no ffmpeg, no microphone, SSH session), sends an
    /// `Error` event and returns.
    pub async fn start_recording(
        &mut self,
        tx: tokio::sync::mpsc::Sender<VoiceEvent>,
    ) -> std::result::Result<(), String> {
        if !self.enabled {
            let _ = tx
                .send(VoiceEvent::Error("Voice mode is not enabled".to_string()))
                .await;
            return Ok(());
        }

        // Lazily create the recorder.
        if self.recorder.is_none() {
            self.recorder = Some(operant_core::voice::create_recorder(self.config.clone()));
        }

        let recorder = self.recorder.as_mut().unwrap();
        match recorder.start(None).await {
            Ok(()) => {
                self.event_tx = Some(tx.clone());
                let _ = tx.send(VoiceEvent::RecordingStarted).await;
                Ok(())
            }
            Err(e) => {
                let _ = tx
                    .send(VoiceEvent::Error(format!(
                        "Failed to start recording: {}",
                        e
                    )))
                    .await;
                Err(format!("Failed to start recording: {}", e))
            }
        }
    }

    #[expect(
        clippy::unwrap_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    /// Stop recording, transcribe the captured audio, and send a
    /// `Transcription` event with the transcript text.
    ///
    /// Returns the raw audio bytes (currently empty — the real audio is
    /// in a temp file managed by the recorder; this return value is kept
    /// for API compatibility with the previous stub).
    pub async fn stop_recording(&mut self) -> std::result::Result<Vec<u8>, String> {
        let recorder = match self.recorder.as_mut() {
            Some(r) => r,
            None => return Ok(vec![]),
        };

        let audio_path = match recorder.stop().await {
            Ok(Some(path)) => path,
            Ok(None) => {
                if let Some(tx) = &self.event_tx {
                    let _ = tx
                        .send(VoiceEvent::Error("No audio captured".to_string()))
                        .await;
                }
                return Ok(vec![]);
            }
            Err(e) => {
                if let Some(tx) = &self.event_tx {
                    let _ = tx
                        .send(VoiceEvent::Error(format!(
                            "Failed to stop recording: {}",
                            e
                        )))
                        .await;
                }
                return Err(format!("Failed to stop recording: {}", e));
            }
        };

        if let Some(tx) = &self.event_tx {
            let _ = tx.send(VoiceEvent::RecordingStopped).await;
        }

        // Lazily create the STT engine.
        if self.stt_engine.is_none() {
            match operant_core::voice::create_stt_engine(&self.config) {
                Ok(engine) => self.stt_engine = Some(engine),
                Err(e) => {
                    if let Some(tx) = &self.event_tx {
                        let _ = tx
                            .send(VoiceEvent::Error(format!("STT engine init failed: {}", e)))
                            .await;
                    }
                    return Ok(vec![]);
                }
            }
        }

        let stt_engine = self.stt_engine.as_ref().unwrap();
        match operant_core::voice::transcribe_recording(&audio_path, stt_engine.as_ref()).await {
            Ok(result) => {
                if result.success && !result.transcript.is_empty() {
                    if let Some(tx) = &self.event_tx {
                        let _ = tx
                            .send(VoiceEvent::Transcription(result.transcript.clone()))
                            .await;
                        let _ = tx
                            .send(VoiceEvent::TranscriptReady(result.transcript))
                            .await;
                    }
                } else if let Some(tx) = &self.event_tx {
                    let _ = tx
                        .send(VoiceEvent::Error(
                            result
                                .error
                                .unwrap_or_else(|| "Empty transcript".to_string()),
                        ))
                        .await;
                }
            }
            Err(e) => {
                if let Some(tx) = &self.event_tx {
                    let _ = tx
                        .send(VoiceEvent::Error(format!("Transcription failed: {}", e)))
                        .await;
                }
            }
        }

        Ok(vec![])
    }
}

impl Default for VoiceRecorder {
    fn default() -> Self {
        Self::new()
    }
}

pub fn global_voice_recorder() -> Arc<Mutex<VoiceRecorder>> {
    use std::sync::OnceLock;
    static INSTANCE: OnceLock<Arc<Mutex<VoiceRecorder>>> = OnceLock::new();
    INSTANCE
        .get_or_init(|| Arc::new(Mutex::new(VoiceRecorder::new())))
        .clone()
}
