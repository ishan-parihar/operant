#![allow(dead_code)] // Foundation modules for future multi-crate extraction — wired in Phase 2I
// voice_capture.rs — Push-to-talk voice capture for TUI.
//
// Integrates with operant_core::voice for audio recording, STT, and TTS.
// Provides PTT (Push-to-Talk) mode with visual feedback in the TUI.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use operant_core::voice::{
    AudioEnvironment, AudioRecorder, SttEngine, VoiceConfig, VoiceError,
    create_recorder, create_stt_engine, detect_audio_environment,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
use tokio::sync::{Mutex, mpsc};

/// Voice capture mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceCaptureMode {
    /// Push-to-talk: hold key to record, release to transcribe
    PushToTalk,
    /// Voice activity detection: auto-start/stop on speech/silence
    VAD,
}

/// Voice capture state
#[derive(Debug, Clone)]
pub struct VoiceCaptureState {
    /// Whether voice capture is available in this environment
    pub available: bool,
    /// Whether voice mode is enabled by user
    pub enabled: bool,
    /// Current capture mode
    pub mode: VoiceCaptureMode,
    /// Whether currently recording
    pub recording: bool,
    /// Recording start time
    pub recording_start: Option<Instant>,
    /// Current RMS level (0.0–32767.0)
    pub current_rms: f64,
    /// Last transcription result
    pub last_transcript: Option<String>,
    /// Last transcription error
    pub last_error: Option<String>,
    /// Environment info
    pub environment: Option<AudioEnvironment>,
    /// Pending transcript to inject into prompt
    pub pending_transcript: Option<String>,
    /// Push-to-talk key (display name)
    pub ptt_key: String,
}

impl Default for VoiceCaptureState {
    fn default() -> Self {
        let env = detect_audio_environment();
        Self {
            available: env.available,
            enabled: false,
            mode: VoiceCaptureMode::PushToTalk,
            recording: false,
            recording_start: None,
            current_rms: 0.0,
            last_transcript: None,
            last_error: None,
            environment: Some(env),
            pending_transcript: None,
            ptt_key: "Alt+V".to_string(),
        }
    }
}

impl VoiceCaptureState {
    /// Create a new state with default PTT key
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the PTT key display name
    pub fn with_ptt_key(mut self, key: &str) -> Self {
        self.ptt_key = key.to_string();
        self
    }

    /// Check if we can start recording
    pub fn can_record(&self) -> bool {
        self.available && self.enabled && !self.recording
    }

    /// Start recording
    pub fn start_recording(&mut self) {
        self.recording = true;
        self.recording_start = Some(Instant::now());
        self.current_rms = 0.0;
        self.last_error = None;
    }

    /// Stop recording
    pub fn stop_recording(&mut self) {
        self.recording = false;
        self.recording_start = None;
    }

    /// Update RMS level
    pub fn update_rms(&mut self, rms: f64) {
        self.current_rms = rms;
    }

    /// Set transcription result
    pub fn set_transcript(&mut self, transcript: String) {
        self.last_transcript = Some(transcript.clone());
        self.pending_transcript = Some(transcript);
    }

    /// Set error
    pub fn set_error(&mut self, error: String) {
        self.last_error = Some(error);
    }

    /// Get recording duration
    pub fn recording_duration(&self) -> Option<Duration> {
        self.recording_start.map(|s| s.elapsed())
    }

    /// Take pending transcript (clears it)
    pub fn take_pending_transcript(&mut self) -> Option<String> {
        self.pending_transcript.take()
    }

    /// Height of the voice indicator when visible
    pub fn indicator_height(&self) -> u16 {
        if self.enabled
            && (self.recording || self.last_transcript.is_some() || self.last_error.is_some())
        {
            3
        } else {
            0
        }
    }
}

/// Voice capture handle for background recording task
pub struct VoiceCaptureHandle {
    /// Channel to send commands to the capture task
    command_tx: mpsc::UnboundedSender<VoiceCommand>,
    /// Channel to receive events from the capture task
    event_rx: mpsc::UnboundedReceiver<VoiceEvent>,
    /// Current state (shared with task)
    state: Arc<Mutex<VoiceCaptureState>>,
}

/// Commands for the voice capture task
enum VoiceCommand {
    StartRecording,
    StopRecording,
    CancelRecording,
    SetEnabled(bool),
    SetMode(VoiceCaptureMode),
    Shutdown,
}

/// Events from the voice capture task
#[derive(Debug, Clone)]
enum VoiceEvent {
    RecordingStarted,
    RecordingStopped(PathBuf),
    RecordingCancelled,
    TranscriptReady(String),
    TranscriptError(String),
    RMSUpdate(f64),
    EnvironmentChanged(AudioEnvironment),
}

impl VoiceCaptureHandle {
    /// Spawn the voice capture background task
    pub fn spawn(config: VoiceConfig) -> Self {
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let state = Arc::new(Mutex::new(VoiceCaptureState::new()));

        // Clone for task
        let task_state = state.clone();
        let task_event_tx = event_tx.clone();
        let task_config = config.clone();

        // Wrap STT engine in Arc for sharing
        let stt_engine = create_stt_engine(&task_config).ok().map(Arc::new);

        tokio::spawn(async move {
            let recorder = Arc::new(Mutex::new(create_recorder(task_config)));
            let stt_engine = stt_engine;

            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    VoiceCommand::StartRecording => {
                        let recorder = recorder.clone();
                        let event_tx = task_event_tx.clone();
                        let recorder_for_rms = recorder.clone();
                        tokio::spawn(async move {
                            let mut rec = recorder.lock().await;
                            if let Err(e) = rec.start(None).await {
                                let _ = event_tx.send(VoiceEvent::TranscriptError(e.to_string()));
                            } else {
                                drop(rec); // Release lock for RMS task
                                let _ = event_tx.send(VoiceEvent::RecordingStarted);
                                // Update RMS periodically
                                let event_tx = event_tx.clone();
                                tokio::spawn(async move {
                                    loop {
                                        let rec = recorder_for_rms.lock().await;
                                        if !rec.is_recording() {
                                            break;
                                        }
                                        let rms = rec.current_rms();
                                        let _ = event_tx.send(VoiceEvent::RMSUpdate(rms));
                                        drop(rec);
                                        tokio::time::sleep(Duration::from_millis(100)).await;
                                    }
                                });
                            }
                        });
                    }
                    VoiceCommand::StopRecording => {
                        let recorder = recorder.clone();
                        let event_tx = task_event_tx.clone();
                        let stt_engine = stt_engine.clone();
                        tokio::spawn(async move {
                            let mut rec = recorder.lock().await;
                            match rec.stop().await {
                                Ok(Some(path)) => {
                                    drop(rec); // Release lock
                                    let _ =
                                        event_tx.send(VoiceEvent::RecordingStopped(path.clone()));
                                    // Transcribe in background
                                    if let Some(engine) = stt_engine {
                                        let event_tx = event_tx.clone();
                                        let engine = engine;
                                        let path_clone = path.clone();
                                        tokio::spawn(async move {
                                            match engine.transcribe(&path_clone).await {
                                                Ok(result) => {
                                                    if result.success
                                                        && !result.transcript.is_empty()
                                                    {
                                                        let _ = event_tx.send(
                                                            VoiceEvent::TranscriptReady(
                                                                result.transcript,
                                                            ),
                                                        );
                                                    } else if let Some(err) = result.error {
                                                        let _ = event_tx
                                                            .send(VoiceEvent::TranscriptError(err));
                                                    }
                                                }
                                                Err(e) => {
                                                    let _ = event_tx.send(
                                                        VoiceEvent::TranscriptError(e.to_string()),
                                                    );
                                                }
                                            }
                                            // Clean up temp file
                                            let _ = tokio::fs::remove_file(&path).await;
                                        });
                                    }
                                }
                                Ok(None) => {
                                    let _ = event_tx.send(VoiceEvent::RecordingCancelled);
                                }
                                Err(e) => {
                                    let _ =
                                        event_tx.send(VoiceEvent::TranscriptError(e.to_string()));
                                }
                            }
                        });
                    }
                    VoiceCommand::CancelRecording => {
                        let recorder = recorder.clone();
                        let event_tx = task_event_tx.clone();
                        tokio::spawn(async move {
                            let mut rec = recorder.lock().await;
                            let _ = rec.cancel().await;
                            let _ = event_tx.send(VoiceEvent::RecordingCancelled);
                        });
                    }
                    VoiceCommand::SetEnabled(enabled) => {
                        let mut s = task_state.lock().await;
                        s.enabled = enabled;
                        if !enabled {
                            s.recording = false;
                            s.recording_start = None;
                        }
                    }
                    VoiceCommand::SetMode(mode) => {
                        let mut s = task_state.lock().await;
                        s.mode = mode;
                    }
                    VoiceCommand::Shutdown => {
                        let recorder = recorder.clone();
                        tokio::spawn(async move {
                            let mut rec = recorder.lock().await;
                            let _ = rec.shutdown().await;
                        });
                        break;
                    }
                }
            }
        });

        Self {
            command_tx: cmd_tx,
            event_rx,
            state,
        }
    }

    /// Start recording (PTT press)
    pub fn start_recording(&self) -> Result<(), VoiceError> {
        self.command_tx
            .send(VoiceCommand::StartRecording)
            .map_err(|_| VoiceError::Recording("Command channel closed".to_string()))
    }

    /// Stop recording (PTT release)
    pub fn stop_recording(&self) -> Result<(), VoiceError> {
        self.command_tx
            .send(VoiceCommand::StopRecording)
            .map_err(|_| VoiceError::Recording("Command channel closed".to_string()))
    }

    /// Cancel recording
    pub fn cancel_recording(&self) -> Result<(), VoiceError> {
        self.command_tx
            .send(VoiceCommand::CancelRecording)
            .map_err(|_| VoiceError::Recording("Command channel closed".to_string()))
    }

    /// Enable/disable voice mode
    pub fn set_enabled(&self, enabled: bool) -> Result<(), VoiceError> {
        self.command_tx
            .send(VoiceCommand::SetEnabled(enabled))
            .map_err(|_| VoiceError::Recording("Command channel closed".to_string()))
    }

    /// Set capture mode
    pub fn set_mode(&self, mode: VoiceCaptureMode) -> Result<(), VoiceError> {
        self.command_tx
            .send(VoiceCommand::SetMode(mode))
            .map_err(|_| VoiceError::Recording("Command channel closed".to_string()))
    }

    /// Poll for events and update state
    pub async fn poll_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            let mut state = self.state.lock().await;
            match event {
                VoiceEvent::RecordingStarted => {
                    state.start_recording();
                }
                VoiceEvent::RecordingStopped(_path) => {
                    state.stop_recording();
                }
                VoiceEvent::RecordingCancelled => {
                    state.stop_recording();
                    state.set_error("Recording cancelled".to_string());
                }
                VoiceEvent::TranscriptReady(transcript) => {
                    state.set_transcript(transcript);
                }
                VoiceEvent::TranscriptError(error) => {
                    state.set_error(error);
                }
                VoiceEvent::RMSUpdate(rms) => {
                    state.update_rms(rms);
                }
                VoiceEvent::EnvironmentChanged(env) => {
                    state.environment = Some(env.clone());
                    state.available = env.available;
                }
            }
        }
    }

    /// Get current state
    pub async fn state(&self) -> VoiceCaptureState {
        self.state.lock().await.clone()
    }

    /// Shutdown the capture task
    pub async fn shutdown(self) {
        let _ = self.command_tx.send(VoiceCommand::Shutdown);
    }
}

/// Render the voice capture indicator (shows recording status, RMS, transcript)
pub fn render_voice_indicator(state: &VoiceCaptureState, area: Rect, buf: &mut Buffer) {
    if !state.enabled || area.height == 0 {
        return;
    }

    let height = state.indicator_height().min(area.height);
    if height == 0 {
        return;
    }

    let indicator_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height,
    };

    Clear.render(indicator_area, buf);

    let mut lines = Vec::new();

    if state.recording {
        let duration = state
            .recording_duration()
            .map(|d| format!("{:.1}s", d.as_secs_f64()))
            .unwrap_or_else(|| "0.0s".to_string());

        // RMS bar visualization
        let rms_normalized = (state.current_rms / 32767.0).min(1.0);
        let bar_width = 20;
        let filled = (rms_normalized * bar_width as f64).round() as usize;
        let bar = "█".repeat(filled) + &"░".repeat(bar_width - filled);

        lines.push(Line::from(vec![
            Span::styled(
                " 🔴 REC ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(duration, Style::default().fg(Color::Yellow)),
            Span::styled("  ", Style::default()),
            Span::styled(bar, Style::default().fg(Color::Green)),
            Span::styled(
                format!("  {:.0}", state.current_rms),
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        lines.push(Line::from(vec![Span::styled(
            format!("  Hold {} to record, release to send", state.ptt_key),
            Style::default().fg(Color::DarkGray),
        )]));
    } else if let Some(transcript) = &state.last_transcript {
        lines.push(Line::from(vec![
            Span::styled(" ✅ ", Style::default().fg(Color::Green)),
            Span::styled("Transcribed: ", Style::default().fg(Color::White)),
            Span::styled(
                transcript,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        lines.push(Line::from(vec![Span::styled(
            "  Press Enter to send, Esc to discard",
            Style::default().fg(Color::DarkGray),
        )]));
    } else if let Some(error) = &state.last_error {
        lines.push(Line::from(vec![
            Span::styled(" ❌ ", Style::default().fg(Color::Red)),
            Span::styled("Error: ", Style::default().fg(Color::White)),
            Span::styled(error, Style::default().fg(Color::Red)),
        ]));

        lines.push(Line::from(vec![Span::styled(
            "  Press Esc to dismiss",
            Style::default().fg(Color::DarkGray),
        )]));
    } else {
        // Voice mode enabled but idle
        let recorder = state
            .environment
            .as_ref()
            .map(|e| e.recorder.as_str())
            .unwrap_or("none");
        lines.push(Line::from(vec![
            Span::styled(" 🎤 ", Style::default().fg(Color::Cyan)),
            Span::styled("Voice mode: ", Style::default().fg(Color::White)),
            Span::styled("Ready (", Style::default().fg(Color::DarkGray)),
            Span::styled(
                state.ptt_key.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(") to record  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("[{}]", recorder),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    Paragraph::new(lines)
        .style(Style::default().bg(Color::Rgb(20, 25, 30)))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .render(indicator_area, buf);
}

/// Render a compact voice status for the footer/status bar
pub fn render_voice_status(state: &VoiceCaptureState, area: Rect, buf: &mut Buffer) {
    if !state.enabled {
        return;
    }

    let text = if state.recording {
        let dur = state
            .recording_duration()
            .map(|d| format!("{:.1}s", d.as_secs_f64()))
            .unwrap_or_else(|| "0.0s".to_string());
        format!("🔴 REC {}", dur)
    } else if state.last_transcript.is_some() {
        "✅ transcribed".to_string()
    } else if state.last_error.is_some() {
        "❌ error".to_string()
    } else {
        format!("🎤 {}", state.ptt_key)
    };

    let style = if state.recording {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else if state.last_transcript.is_some() {
        Style::default().fg(Color::Green)
    } else if state.last_error.is_some() {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Cyan)
    };

    Paragraph::new(text)
        .style(style)
        .alignment(ratatui::layout::Alignment::Right)
        .render(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voice_capture_state_default() {
        let state = VoiceCaptureState::new();
        assert!(!state.enabled);
        assert!(!state.recording);
        assert_eq!(state.ptt_key, "Alt+V");
    }

    #[test]
    fn test_voice_capture_state_recording() {
        let mut state = VoiceCaptureState::new();
        state.enabled = true;
        state.available = true;
        assert!(state.can_record());

        state.start_recording();
        assert!(state.recording);
        assert!(state.recording_start.is_some());

        state.stop_recording();
        assert!(!state.recording);
        assert!(state.recording_start.is_none());
    }

    #[test]
    fn test_voice_capture_state_transcript() {
        let mut state = VoiceCaptureState::new();
        state.set_transcript("Hello world".to_string());
        assert_eq!(state.last_transcript, Some("Hello world".to_string()));
        assert_eq!(state.pending_transcript, Some("Hello world".to_string()));

        let taken = state.take_pending_transcript();
        assert_eq!(taken, Some("Hello world".to_string()));
        assert!(state.pending_transcript.is_none());
    }

    #[test]
    fn test_voice_capture_state_error() {
        let mut state = VoiceCaptureState::new();
        state.set_error("Test error".to_string());
        assert_eq!(state.last_error, Some("Test error".to_string()));
    }
}
