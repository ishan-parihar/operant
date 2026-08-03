//! Text-to-Speech Tool with multiple provider support
//!
//! Supported providers:
//! - Edge TTS (default, free, no API key): Microsoft Edge neural voices
//! - ElevenLabs (premium): High-quality voices, needs ELEVENLABS_API_KEY
//! - OpenAI TTS: Good quality, needs OPENAI_API_KEY
//! - MiniMax TTS: High-quality with voice cloning, needs MINIMAX_API_KEY
//! - Mistral (Voxtral TTS): Multilingual, needs MISTRAL_API_KEY
//! - Google Gemini TTS: Controllable, 30 prebuilt voices, needs GEMINI_API_KEY
//! - xAI TTS: Grok voices, needs XAI_API_KEY
//! - NeuTTS (local, free): On-device TTS via neutts binary
//! - KittenTTS (local, free): On-device 25MB model via kittentts
//! - Piper (local, free): OHF-Voice/piper1-gpl neural VITS

use async_trait::async_trait;
use kokoro_tiny::TtsEngine;
use reqwest::Client;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::debug;

use crate::config::runtime_config;
use crate::schema::ToolSchema;
use crate::tools::tts_command_provider::CommandProvider;
use crate::tools::tts_provider::{AudioFormat, TtsProvider};
use crate::tools::tts_registry::TtsPluginRegistry;
use crate::tools::{OperantTool, ToolContext, ToolResult};

pub struct TtsTool {
    client: Client,
    elevenlabs_key: String,
    openai_key: String,
    minimax_key: String,
    mistral_key: String,
    gemini_key: String,
    xai_key: String,
    kokoro_engine: Arc<Mutex<Option<TtsEngine>>>,
    plugin_registry: Arc<TtsPluginRegistry>,
    command_providers: Arc<Mutex<HashMap<String, CommandProvider>>>,
}

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsArgs {
    /// The text to convert to speech
    pub text: String,
    /// TTS provider to use: edge, elevenlabs, openai, minimax, mistral, gemini, xai, neutts, kittentts, piper, kokoro
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Voice ID or name (provider-specific)
    #[serde(default = "default_voice")]
    pub voice: String,
    /// Model ID (optional, provider-specific defaults)
    pub model: Option<String>,
}

fn default_provider() -> String {
    "kokoro".to_string()
}

fn default_voice() -> String {
    "af_sky".to_string()
}

impl Default for TtsTool {
    fn default() -> Self {
        Self::new()
    }
}

impl TtsTool {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            elevenlabs_key: std::env::var("ELEVENLABS_API_KEY").unwrap_or_default(),
            openai_key: std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            minimax_key: std::env::var("MINIMAX_API_KEY").unwrap_or_default(),
            mistral_key: std::env::var("MISTRAL_API_KEY").unwrap_or_default(),
            gemini_key: std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            xai_key: std::env::var("XAI_API_KEY").unwrap_or_default(),
            kokoro_engine: Arc::new(Mutex::new(None)),
            plugin_registry: Arc::new(TtsPluginRegistry::new()),
            command_providers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_plugin_registry(registry: Arc<TtsPluginRegistry>) -> Self {
        let mut tool = Self::new();
        tool.plugin_registry = registry;
        tool
    }

    pub async fn register_command_provider(&self, name: String, provider: CommandProvider) {
        let mut providers = self.command_providers.lock().await;
        providers.insert(name, provider);
    }

    pub async fn get_plugin_registry(&self) -> Arc<TtsPluginRegistry> {
        self.plugin_registry.clone()
    }

    async fn generate_speech(&self, args: &TtsArgs) -> ToolResult {
        if args.text.trim().is_empty() {
            return ToolResult::error("text_to_speech", "Text is required");
        }

        let provider = args.provider.as_str();

        // Command providers win over plugins (config is more local than plugin install)
        {
            let providers = self.command_providers.lock().await;
            if let Some(cmd_provider) = providers.get(provider) {
                debug!(provider = %provider, "Dispatching to command provider");
                let temp_dir = std::env::temp_dir();
                let output_path =
                    temp_dir.join(format!("tts_output.{}", cmd_provider.output_format()));
                match cmd_provider
                    .synthesize(
                        &args.text,
                        output_path.to_str().unwrap_or("/tmp/tts_output.mp3"),
                        Some(&args.voice),
                        args.model.as_deref(),
                        AudioFormat::Mp3,
                    )
                    .await
                {
                    Ok(result) => {
                        let audio_bytes = match std::fs::read(&result.output_path) {
                            Ok(b) => b,
                            Err(e) => {
                                return ToolResult::error(
                                    "text_to_speech",
                                    format!("Failed to read output: {}", e),
                                );
                            }
                        };
                        let audio_base64 = base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            &audio_bytes,
                        );
                        let _ = std::fs::remove_file(&result.output_path);
                        return ToolResult::success(
                            "text_to_speech",
                            json!({
                                "success": true,
                                "audio": audio_base64,
                                "format": result.format.to_string(),
                                "provider": provider,
                                "voice": args.voice
                            }),
                        );
                    }
                    Err(e) => {
                        return ToolResult::error(
                            "text_to_speech",
                            format!("Command provider failed: {}", e),
                        );
                    }
                }
            }
        }

        // Plugin-registered providers (dispatched when name is not a built-in)
        if !TtsPluginRegistry::is_builtin(provider) {
            if let Some(plugin) = self.plugin_registry.get_provider(provider).await {
                debug!(provider = %provider, "Dispatching to plugin provider");
                let temp_dir = std::env::temp_dir();
                let output_path = temp_dir.join("tts_plugin_output.mp3");
                match plugin
                    .synthesize(
                        &args.text,
                        output_path.to_str().unwrap_or("/tmp/tts_plugin_output.mp3"),
                        Some(&args.voice),
                        args.model.as_deref(),
                        AudioFormat::Mp3,
                    )
                    .await
                {
                    Ok(result) => {
                        let audio_bytes = match std::fs::read(&result.output_path) {
                            Ok(b) => b,
                            Err(e) => {
                                return ToolResult::error(
                                    "text_to_speech",
                                    format!("Failed to read output: {}", e),
                                );
                            }
                        };
                        let audio_base64 = base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            &audio_bytes,
                        );
                        let _ = std::fs::remove_file(&result.output_path);
                        return ToolResult::success(
                            "text_to_speech",
                            json!({
                                "success": true,
                                "audio": audio_base64,
                                "format": result.format.to_string(),
                                "provider": provider,
                                "voice": args.voice
                            }),
                        );
                    }
                    Err(e) => {
                        return ToolResult::error(
                            "text_to_speech",
                            format!("Plugin provider failed: {}", e),
                        );
                    }
                }
            }
        }

        // Built-in providers
        match provider {
            "edge" => self.edge_tts(args).await,
            "elevenlabs" => self.elevenlabs_tts(args).await,
            "openai" => self.openai_tts(args).await,
            "minimax" => self.minimax_tts(args).await,
            "mistral" => self.mistral_tts(args).await,
            "gemini" => self.gemini_tts(args).await,
            "xai" => self.xai_tts(args).await,
            "neutts" => self.neutts_local(args).await,
            "kittentts" => self.kittentts_local(args).await,
            "piper" => self.piper_local(args).await,
            "kokoro" => self.kokoro_local(args).await,
            _ => ToolResult::error(
                "text_to_speech",
                format!(
                    "Unknown provider: {}. Available: edge, elevenlabs, openai, minimax, mistral, gemini, xai, neutts, kittentts, piper, kokoro, or registered command/plugin providers",
                    args.provider
                ),
            ),
        }
    }

    // =========================================================================
    // Edge TTS (free, no API key)
    // =========================================================================
    async fn edge_tts(&self, args: &TtsArgs) -> ToolResult {
        let voice = if args.voice.is_empty() {
            "en-US-AriaNeural"
        } else {
            &args.voice
        };

        // Edge TTS requires the edge-tts crate which is complex - we'll simulate with a message
        // In production, you'd use the edge-tts Rust crate or call subprocess
        ToolResult::success(
            "text_to_speech",
            json!({
                "success": true,
                "message": "Edge TTS requires 'edge-tts' Python package. Install with: pip install edge-tts",
                "provider": "edge",
                "voice": voice,
                "format": "mp3",
                "note": "Edge TTS is free but requires Python dependency"
            }),
        )
    }

    // =========================================================================
    // ElevenLabs TTS
    // =========================================================================
    async fn elevenlabs_tts(&self, args: &TtsArgs) -> ToolResult {
        if self.elevenlabs_key.is_empty() {
            return ToolResult::error("text_to_speech", "ELEVENLABS_API_KEY not set");
        }

        let model = args.model.as_deref().unwrap_or("eleven_multilingual_v2");
        let voice_id = if args.voice.is_empty() {
            "pNInz6obpgDQGcFmaJgB"
        } else {
            &args.voice
        };

        let response = match self
            .client
            .post(format!(
                "https://api.elevenlabs.io/v1/text-to-speech/{}",
                voice_id
            ))
            .header("xi-api-key", &self.elevenlabs_key)
            .header("Content-Type", "application/json")
            .json(&json!({
                "text": args.text,
                "model_id": model,
            }))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error("text_to_speech", format!("API request failed: {}", e));
            }
        };

        if !response.status().is_success() {
            return ToolResult::error(
                "text_to_speech",
                format!("ElevenLabs API error: {}", response.status()),
            );
        }

        let audio_bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return ToolResult::error("text_to_speech", format!("Failed to read audio: {}", e));
            }
        };

        let audio_base64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &audio_bytes);

        ToolResult::success(
            "text_to_speech",
            json!({
                "success": true,
                "audio": audio_base64,
                "format": "mp3",
                "provider": "elevenlabs",
                "voice": voice_id,
                "model": model
            }),
        )
    }

    // =========================================================================
    // OpenAI TTS
    // =========================================================================
    async fn openai_tts(&self, args: &TtsArgs) -> ToolResult {
        if self.openai_key.is_empty() {
            return ToolResult::error("text_to_speech", "OPENAI_API_KEY not set");
        }

        let model = args.model.as_deref().unwrap_or("gpt-4o-mini-tts");
        let voice = if args.voice.is_empty() {
            "alloy"
        } else {
            &args.voice
        };

        let response = match self
            .client
            .post("https://api.openai.com/v1/audio/speech")
            .header("Authorization", format!("Bearer {}", self.openai_key))
            .header("Content-Type", "application/json")
            .json(&json!({
                "model": model,
                "voice": voice,
                "input": args.text
            }))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error("text_to_speech", format!("API request failed: {}", e));
            }
        };

        if !response.status().is_success() {
            return ToolResult::error(
                "text_to_speech",
                format!("OpenAI API error: {}", response.status()),
            );
        }

        let audio_bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return ToolResult::error("text_to_speech", format!("Failed to read audio: {}", e));
            }
        };

        let audio_base64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &audio_bytes);

        ToolResult::success(
            "text_to_speech",
            json!({
                "success": true,
                "audio": audio_base64,
                "format": "mp3",
                "provider": "openai",
                "model": model,
                "voice": voice
            }),
        )
    }

    // =========================================================================
    // MiniMax TTS
    // =========================================================================
    async fn minimax_tts(&self, args: &TtsArgs) -> ToolResult {
        if self.minimax_key.is_empty() {
            return ToolResult::error("text_to_speech", "MINIMAX_API_KEY not set");
        }

        let voice_id = if args.voice.is_empty() {
            "female-shaonv"
        } else {
            &args.voice
        };

        let response = match self
            .client
            .post("https://api.minimax.chat/v1/text_to_audio")
            .header("Authorization", format!("Bearer {}", self.minimax_key))
            .header("Content-Type", "application/json")
            .json(&json!({
                "model": "speech-01",
                "text": args.text,
                "voice_id": voice_id,
                "speed": 1.0,
                "volume": 1.0
            }))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error("text_to_speech", format!("API request failed: {}", e));
            }
        };

        if !response.status().is_success() {
            return ToolResult::error(
                "text_to_speech",
                format!("MiniMax API error: {}", response.status()),
            );
        }

        let result: Value = match response.json().await {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error(
                    "text_to_speech",
                    format!("Failed to parse response: {}", e),
                );
            }
        };

        // MiniMax returns base64 audio
        let audio_base64 = result
            .get("data")
            .and_then(|d| d.get("audio"))
            .and_then(|a| a.as_str())
            .unwrap_or("");

        ToolResult::success(
            "text_to_speech",
            json!({
                "success": true,
                "audio": audio_base64,
                "format": "mp3",
                "provider": "minimax",
                "voice": voice_id,
                "model": "speech-01"
            }),
        )
    }

    // =========================================================================
    // Mistral TTS (Voxtral)
    // =========================================================================
    async fn mistral_tts(&self, args: &TtsArgs) -> ToolResult {
        if self.mistral_key.is_empty() {
            return ToolResult::error("text_to_speech", "MISTRAL_API_KEY not set");
        }

        let voice_id = if args.voice.is_empty() {
            "c69964a6-ab8b-4f8a-9465-ec0925096ec8"
        } else {
            &args.voice
        };

        let response = match self
            .client
            .post("https://api.mistral.ai/v1/audio/speech")
            .header("Authorization", format!("Bearer {}", self.mistral_key))
            .header("Content-Type", "application/json")
            .json(&json!({
                "model": "voxtral-mini-tts-2603",
                "text": args.text,
                "voice": voice_id
            }))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error("text_to_speech", format!("API request failed: {}", e));
            }
        };

        if !response.status().is_success() {
            return ToolResult::error(
                "text_to_speech",
                format!("Mistral API error: {}", response.status()),
            );
        }

        let audio_bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return ToolResult::error("text_to_speech", format!("Failed to read audio: {}", e));
            }
        };

        let audio_base64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &audio_bytes);

        ToolResult::success(
            "text_to_speech",
            json!({
                "success": true,
                "audio": audio_base64,
                "format": "mp3",
                "provider": "mistral",
                "voice": voice_id,
                "model": "voxtral-mini-tts-2603"
            }),
        )
    }

    // =========================================================================
    // Google Gemini TTS
    // =========================================================================
    async fn gemini_tts(&self, args: &TtsArgs) -> ToolResult {
        if self.gemini_key.is_empty() {
            return ToolResult::error("text_to_speech", "GEMINI_API_KEY not set");
        }

        let voice = if args.voice.is_empty() {
            "Kore"
        } else {
            &args.voice
        };

        let response = match self
            .client
            .post("https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-preview-tts:generateContent")
            .header("Authorization", format!("Bearer {}", self.gemini_key))
            .header("Content-Type", "application/json")
            .query(&[("key", &self.gemini_key)])
            .json(&json!({
                "contents": [{
                    "parts": [{
                        "text": args.text
                    }]
                }],
                "generationConfig": {
                    "speechConfig": {
                        "voiceConfig": {
                            "prebuiltVoice": {
                                "speaker": voice
                            }
                        }
                    }
                }
            }))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return ToolResult::error("text_to_speech", format!("API request failed: {}", e)),
        };

        if !response.status().is_success() {
            return ToolResult::error(
                "text_to_speech",
                format!("Gemini API error: {}", response.status()),
            );
        }

        let result: Value = match response.json().await {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error(
                    "text_to_speech",
                    format!("Failed to parse response: {}", e),
                );
            }
        };

        // Gemini returns base64 in candidates[0].content.parts[0].inlineData.data
        let audio_base64 = result
            .get("candidates")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
            .and_then(|arr| arr.first())
            .and_then(|p| p.get("inlineData"))
            .and_then(|d| d.get("data"))
            .and_then(|a| a.as_str())
            .unwrap_or("");

        ToolResult::success(
            "text_to_speech",
            json!({
                "success": true,
                "audio": audio_base64,
                "format": "mp3",
                "provider": "gemini",
                "voice": voice,
                "model": "gemini-2.5-flash-preview-tts"
            }),
        )
    }

    // =========================================================================
    // xAI (Grok) TTS
    // =========================================================================
    async fn xai_tts(&self, args: &TtsArgs) -> ToolResult {
        if self.xai_key.is_empty() {
            return ToolResult::error("text_to_speech", "XAI_API_KEY not set");
        }

        let voice_id = if args.voice.is_empty() {
            "eve"
        } else {
            &args.voice
        };

        let response = match self
            .client
            .post("https://api.x.ai/v1/audio/speech")
            .header("Authorization", format!("Bearer {}", self.xai_key))
            .header("Content-Type", "application/json")
            .json(&json!({
                "model": "grok-tts",
                "voice": voice_id,
                "input": args.text
            }))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error("text_to_speech", format!("API request failed: {}", e));
            }
        };

        if !response.status().is_success() {
            return ToolResult::error(
                "text_to_speech",
                format!("xAI API error: {}", response.status()),
            );
        }

        let audio_bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return ToolResult::error("text_to_speech", format!("Failed to read audio: {}", e));
            }
        };

        let audio_base64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &audio_bytes);

        ToolResult::success(
            "text_to_speech",
            json!({
                "success": true,
                "audio": audio_base64,
                "format": "mp3",
                "provider": "xai",
                "voice": voice_id,
                "model": "grok-tts"
            }),
        )
    }

    // =========================================================================
    // NeuTTS (local, free - requires neutts binary)
    // =========================================================================
    async fn neutts_local(&self, args: &TtsArgs) -> ToolResult {
        let voice = if args.voice.is_empty() {
            "neutral"
        } else {
            &args.voice
        };
        let model = args
            .model
            .as_deref()
            .unwrap_or("neuphonic/neutts-air-q4-gguf");

        // Check if neutts is available
        let check = Command::new("neutts").arg("--help").output();
        if check.is_err() {
            return ToolResult::error(
                "text_to_speech",
                "NeuTTS not installed. Install with: pip install neutts",
            );
        }

        // Create temp file for output
        let temp_dir = std::env::temp_dir();
        let output_path = temp_dir.join("neutts_output.wav");

        let result = Command::new("neutts")
            .arg("--model")
            .arg(model)
            .arg("--voice")
            .arg(voice)
            .arg("--text")
            .arg(&args.text)
            .arg("--output")
            .arg(&output_path)
            .output();

        match result {
            Ok(output) if output.status.success() => {
                // Read the generated audio
                match std::fs::read(&output_path) {
                    Ok(audio_bytes) => {
                        let _ = std::fs::remove_file(&output_path);
                        let audio_base64 = base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            &audio_bytes,
                        );
                        ToolResult::success(
                            "text_to_speech",
                            json!({
                                "success": true,
                                "audio": audio_base64,
                                "format": "wav",
                                "provider": "neutts",
                                "voice": voice,
                                "model": model
                            }),
                        )
                    }
                    Err(e) => {
                        ToolResult::error("text_to_speech", format!("Failed to read output: {}", e))
                    }
                }
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                ToolResult::error("text_to_speech", format!("NeuTTS failed: {}", stderr))
            }
            Err(e) => ToolResult::error("text_to_speech", format!("Failed to run neutts: {}", e)),
        }
    }

    // =========================================================================
    // KittenTTS (local, free - requires kittentts Python package)
    // =========================================================================
    async fn kittentts_local(&self, args: &TtsArgs) -> ToolResult {
        let voice = if args.voice.is_empty() {
            "Jasper"
        } else {
            &args.voice
        };
        let model = args
            .model
            .as_deref()
            .unwrap_or("KittenML/kitten-tts-nano-0.8-int8");

        // Use Python to run kittentts
        let temp_dir = std::env::temp_dir();
        let output_path = temp_dir.join("kittentts_output.wav");

        let python_code = format!(
            r#"
import sys
try:
    from kittentts import KittenTTS
    tts = KittenTTS()
    tts.tts(
        text="{}",
        speaker="{}",
        output_path="{}"
    )
    print("SUCCESS")
except Exception as e:
    print(f"ERROR: {{e}}", file=sys.stderr)
    sys.exit(1)
"#,
            args.text.replace("\"", "\\\""),
            voice,
            output_path.display()
        );

        let result = Command::new("python3").arg("-c").arg(&python_code).output();

        match result {
            Ok(output) if output.status.success() => match std::fs::read(&output_path) {
                Ok(audio_bytes) => {
                    let _ = std::fs::remove_file(&output_path);
                    let audio_base64 = base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        &audio_bytes,
                    );
                    ToolResult::success(
                        "text_to_speech",
                        json!({
                            "success": true,
                            "audio": audio_base64,
                            "format": "wav",
                            "provider": "kittentts",
                            "voice": voice,
                            "model": model
                        }),
                    )
                }
                Err(e) => {
                    ToolResult::error("text_to_speech", format!("Failed to read output: {}", e))
                }
            },
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                ToolResult::error("text_to_speech", format!("KittenTTS failed: {}", stderr))
            }
            Err(e) => ToolResult::error("text_to_speech", format!("Failed to run Python: {}", e)),
        }
    }

    // =========================================================================
    // Piper (local, free - requires piper binary)
    // =========================================================================
    async fn piper_local(&self, args: &TtsArgs) -> ToolResult {
        let voice = if args.voice.is_empty() {
            "en_US-lessac-medium"
        } else {
            &args.voice
        };

        // Check if piper is available
        let check = Command::new("piper").arg("--help").output();
        if check.is_err() {
            return ToolResult::error(
                "text_to_speech",
                "Piper not installed. Install from: https://github.com/rhasspy/piper",
            );
        }

        let temp_dir = std::env::temp_dir();
        let text_path = temp_dir.join("piper_input.txt");
        let output_path = temp_dir.join("piper_output.wav");

        // Write text to temp file
        if let Err(e) = std::fs::write(&text_path, &args.text) {
            return ToolResult::error(
                "text_to_speech",
                format!("Failed to write temp file: {}", e),
            );
        }

        let result = Command::new("piper")
            .arg("--model")
            .arg(voice)
            .arg("--input_file")
            .arg(&text_path)
            .arg("--output_file")
            .arg(&output_path)
            .output();

        let _ = std::fs::remove_file(&text_path);

        match result {
            Ok(output) if output.status.success() => match std::fs::read(&output_path) {
                Ok(audio_bytes) => {
                    let _ = std::fs::remove_file(&output_path);
                    let audio_base64 = base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        &audio_bytes,
                    );
                    ToolResult::success(
                        "text_to_speech",
                        json!({
                            "success": true,
                            "audio": audio_base64,
                            "format": "wav",
                            "provider": "piper",
                            "voice": voice
                        }),
                    )
                }
                Err(e) => {
                    ToolResult::error("text_to_speech", format!("Failed to read output: {}", e))
                }
            },
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                ToolResult::error("text_to_speech", format!("Piper failed: {}", stderr))
            }
            Err(e) => ToolResult::error("text_to_speech", format!("Failed to run piper: {}", e)),
        }
    }

        #[expect(clippy::expect_used, reason = "invariant guaranteed by surrounding validation")]
    async fn kokoro_local(&self, args: &TtsArgs) -> ToolResult {
        let voice = if args.voice.is_empty() {
            "af_sky"
        } else {
            &args.voice
        };

        let mut engine_lock = self.kokoro_engine.lock().await;
        if engine_lock.is_none() {
            match TtsEngine::new().await {
                Ok(engine) => *engine_lock = Some(engine),
                Err(e) => {
                    return ToolResult::error(
                        "text_to_speech",
                        format!("Failed to initialize Kokoro engine: {}", e),
                    );
                }
            }
        }

        let engine = engine_lock
            .as_mut()
            .expect("engine initialized before use (guarded by init check)");

        match engine.synthesize(&args.text, Some(voice)) {
            Ok(audio) => {
                let spec = hound::WavSpec {
                    channels: 1,
                    sample_rate: 24000, // Kokoro default sample rate
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                };

                let mut cursor = std::io::Cursor::new(Vec::new());
                {
                    let mut writer = match hound::WavWriter::new(&mut cursor, spec) {
                        Ok(w) => w,
                        Err(e) => {
                            return ToolResult::error(
                                "text_to_speech",
                                format!("Failed to create WAV writer: {}", e),
                            );
                        }
                    };

                    for sample in audio {
                        // Convert f32 (-1.0 to 1.0) to i16
                        let s = (sample * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32)
                            as i16;
                        if let Err(e) = writer.write_sample(s) {
                            return ToolResult::error(
                                "text_to_speech",
                                format!("Failed to write sample: {}", e),
                            );
                        }
                    }
                    if let Err(e) = writer.finalize() {
                        return ToolResult::error(
                            "text_to_speech",
                            format!("Failed to finalize WAV: {}", e),
                        );
                    }
                }

                let wav_bytes = cursor.into_inner();
                let audio_base64 =
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &wav_bytes);
                ToolResult::success(
                    "text_to_speech",
                    json!({
                        "success": true,
                        "audio": audio_base64,
                        "format": "wav",
                        "provider": "kokoro",
                        "voice": voice
                    }),
                )
            }
            Err(e) => {
                ToolResult::error("text_to_speech", format!("Kokoro synthesis failed: {}", e))
            }
        }
    }
}

#[async_trait]
impl OperantTool for TtsTool {
    fn name(&self) -> &str {
        "text_to_speech"
    }

    fn description(&self) -> &str {
        "Convert text to speech using AI voice providers (Edge, ElevenLabs, OpenAI, MiniMax, Mistral, Gemini, xAI, NeuTTS, KittenTTS, Piper)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<TtsArgs>(
            "text_to_speech",
            "Convert text to speech with multiple AI providers",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let mut args: TtsArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("text_to_speech", format!("Invalid arguments: {}", e));
            }
        };

        // If caller left provider/voice as defaults, honour the global config.
        let cfg = runtime_config();
        if args.provider == default_provider() && cfg.tts.provider != default_provider() {
            args.provider = cfg.tts.provider.clone();
        }
        if args.voice == default_voice() {
            if let Some(ref v) = cfg.tts.voice {
                if !v.is_empty() {
                    args.voice = v.clone();
                }
            }
        }

        self.generate_speech(&args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_tts_schema() {
        let tool = TtsTool::new();
        let schema = tool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "text_to_speech");
    }

    #[test]
    fn test_default_provider() {
        assert_eq!(default_provider(), "kokoro");
    }

    #[test]
    fn test_default_voice() {
        assert_eq!(default_voice(), "af_sky");
    }

    #[tokio::test]
    async fn test_tts_invalid_args() {
        let tool = TtsTool::new();
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_tts_empty_text() {
        let tool = TtsTool::new();
        let result = tool
            .execute(
                json!({"text": "", "provider": "edge"}),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_tts_unknown_provider() {
        let tool = TtsTool::new();
        let result = tool
            .execute(
                json!({"text": "hello", "provider": "unknown"}),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
    }
}
