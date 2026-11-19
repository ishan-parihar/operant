//! Text-to-Speech Tool with multiple provider support
//!
//! Supported providers (11 built-ins, dispatched by `BUILTIN_PROVIDERS`):
//!   - edge (keyless, via the `edge-tts` CLI; NOT local — it calls Microsoft)
//!   - elevenlabs (ELEVENLABS_API_KEY), openai (OPENAI_API_KEY),
//!     minimax (MINIMAX_API_KEY), mistral (MISTRAL_API_KEY),
//!     gemini (GEMINI_API_KEY), xai (XAI_API_KEY)
//!   - neutts / kittentts / piper / phonon (local, no key; `phonon` is default)
//!
//! Voice handling: `voice` is optional. The schema default is the sentinel
//! `"default"`, which is a real voice name ONLY for Phonon, so
//! [`resolve_voice`] maps the sentinel (or an empty string) to the provider's
//! own default rather than forwarding it to a provider that would reject it.
//!
//! Two invariants the tool tests hold:
//!   - a SUCCESS carries decodable audio (`no_builtin_provider_reports_success_without_audio`)
//!   - a provider that cannot be reached FAILS, never claims success silently

use async_trait::async_trait;
use reqwest::Client;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::process::Command;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config::runtime_config;
use crate::schema::ToolSchema;
use crate::tools::phonon;
use crate::tools::{OperantTool, ToolContext, ToolResult};

pub struct TtsTool {
    client: Client,
    elevenlabs_key: String,
    openai_key: String,
    minimax_key: String,
    mistral_key: String,
    gemini_key: String,
    xai_key: String,
    /// Phonon (gradium/phonon-31m-en) engine, loaded lazily on first use.
    /// `ptts::Synth` is a single-stream, CPU-resident model; the tool holds it
    /// behind a `tokio::Mutex` and every synthesis borrows it mutably. This is
    /// the local, offline default voice — it replaces the old Kokoro engine.
    phonon: Arc<Mutex<Option<phonon::PhononSynth>>>,
}

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsArgs {
    /// The text to convert to speech
    pub text: String,
    /// TTS provider to use: edge, elevenlabs, openai, minimax, mistral, gemini, xai, neutts, kittentts, piper, phonon
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Voice ID or name (provider-specific)
    #[serde(default = "default_voice")]
    pub voice: String,
    /// Model ID (optional, provider-specific defaults)
    pub model: Option<String>,
}

fn default_provider() -> String {
    default_provider_name().to_string()
}

/// The built-in provider name this build dispatches to when the config names
/// nothing usable.
///
/// Public because a caller that must REWRITE a retired config value needs the
/// same answer the dispatch uses. A second hardcoded `"phonon"` in the CLI
/// would drift the moment the default moves, and the migration would then
/// quietly write a name that is not the default.
pub fn default_provider_name() -> &'static str {
    "phonon"
}

fn default_voice() -> String {
    "default".to_string()
}

/// MiniMax's current synchronous speech endpoint.
///
/// The old pin was `https://api.minimax.chat/v1/text_to_audio` with the model
/// `speech-01`. The vendor's current API is `t2a_v2` on `api.minimax.io`, and
/// `speech-01` is no longer a valid model id — only `speech-01-hd` /
/// `speech-01-turbo` and the 2.x generations are. A request on the old pair
/// fails outright rather than degrading.
const MINIMAX_SPEECH_URL: &str = "https://api.minimax.io/v1/t2a_v2";
const MINIMAX_DEFAULT_MODEL: &str = "speech-2.8-hd";
/// A voice id the current API ships, so a caller who names none gets a real
/// one instead of a name only the retired generation accepted.
const MINIMAX_DEFAULT_VOICE: &str = "English_expressive_narrator";

/// Gemini's current TTS model.
///
/// The old pin was `gemini-2.5-flash-preview-tts`, a preview generation with a
/// published shutdown window. Two things changed beyond the name: 3.x takes the
/// input text strictly as a verbatim transcript (turn-level directions move to
/// `speech_metadata`), and the voice field is `prebuiltVoiceConfig.voiceName`.
const GEMINI_DEFAULT_MODEL: &str = "gemini-3.8-flash-tts";
const GEMINI_DEFAULT_VOICE: &str = "Kore";

/// Decode the hex payload `t2a_v2` returns under `output_format: "hex"`.
///
/// This used to be handed back verbatim as if it were base64, so the caller
/// received a string that no audio decoder could read while the tool reported
/// success. Returns `None` on odd length or a non-hex digit, so a malformed
/// payload is an error rather than silently-truncated audio.
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if !s.len().is_multiple_of(2) {
        return None;
    }
    s.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| {
            let hi = (pair[0] as char).to_digit(16)?;
            let lo = (pair[1] as char).to_digit(16)?;
            Some(((hi << 4) | lo) as u8)
        })
        .collect()
}

/// The voice this provider uses when the caller named none.
///
/// `default_voice()` is the SCHEMA default, and it means "whatever this
/// provider's own default is" — but only Phonon has a voice literally named
/// `default`. Every other provider validates its voice id, so sending the
/// sentinel through produced a hard API error on every turn that omitted
/// `voice` (the overwhelmingly common case). Measured on `edge`:
/// `ValueError: Invalid voice 'default'.`. Empty and `default` are the same
/// request, so both resolve here rather than at eleven call sites.
fn resolve_voice<'a>(requested: &'a str, provider_default: &'a str) -> &'a str {
    if requested.is_empty() || requested == default_voice() {
        provider_default
    } else {
        requested
    }
}

/// Resolve how to invoke `edge-tts`, or `None` when no way to run it exists.
///
/// The package installs a `edge-tts` console script; when that is not on
/// `PATH` but `python3` is, the module form (`python3 -m edge_tts`) still
/// works, because the package may be installed for a python the script was
/// never linked for. Returns the command plus any leading arguments, so the
/// two forms are one call shape rather than two code paths.
fn edge_tts_command() -> Option<(&'static str, &'static [&'static str])> {
    if command_on_path("edge-tts") {
        return Some(("edge-tts", &[]));
    }
    if command_on_path("python3") {
        return Some(("python3", &["-m", "edge_tts"]));
    }
    None
}

/// Whether `name` resolves to a file on `PATH`.
///
/// This is a cheap probe (`is_file`), not a `--help` spawn: the old piper
/// probe shelled out on every call, which costs a process for a question the
/// filesystem answers.
fn command_on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(name).is_file())
}

/// The built-in provider names this tool can dispatch to.
///
/// One list, so the dispatch `match` below, the error message and
/// [`resolve_provider_name`] cannot drift apart. A name outside it is a
/// config that named a provider this build does not have.
const BUILTIN_PROVIDERS: &[&str] = &[
    "edge",
    "elevenlabs",
    "openai",
    "minimax",
    "mistral",
    "gemini",
    "xai",
    "neutts",
    "kittentts",
    "piper",
    "phonon",
];

/// The provider names this tool can dispatch to.
///
/// One exported list, so the wizard's menu (`cmd_setup::tts_provider_menu`)
/// and the unknown-provider message cannot drift from the dispatch arms. A
/// menu entry the tool cannot reach is a wizard that lies, and a tool
/// provider the wizard omits is one the operator cannot select.
pub fn builtin_providers() -> &'static [&'static str] {
    BUILTIN_PROVIDERS
}

/// Map a configured provider name onto one this build can dispatch to.
///
/// A name this build does not have degrades to the default with one warning,
/// the same posture the memory provider takes for a retired name. That covers
/// both a retired name (`kokoro`, the default until iter-2513) and a typo:
/// an operator whose `operant.toml` still says `provider = "kokoro"` would
/// otherwise get "Unknown provider" on every speech request — a config that
/// parsed yesterday failing today.
///
/// The tool's own `provider` ARGUMENT is deliberately NOT resolved: a plugin
/// or command provider may register under a name outside [`BUILTIN_PROVIDERS`],
/// so only the value that came from config is resolved here.
fn resolve_provider_name(name: &str) -> String {
    if BUILTIN_PROVIDERS.contains(&name) {
        return name.to_string();
    }
    if !name.is_empty() {
        tracing::warn!(
            provider = %name,
            replacement = %default_provider(),
            "TTS provider is unknown; using the default"
        );
    }
    default_provider()
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
            // `cmd_auth` writes `GOOGLE_API_KEY` when the operator runs
            // `operant auth add google` (cmd_auth.rs), while this provider read
            // only `GEMINI_API_KEY`. An operator who added a Google key through
            // the tool's own auth command therefore got "GEMINI_API_KEY not
            // set" for a key they had already configured. Both names are
            // accepted; the service-specific one wins when both are present.
            gemini_key: std::env::var("GEMINI_API_KEY")
                .ok()
                .filter(|k| !k.is_empty())
                .or_else(|| std::env::var("GOOGLE_API_KEY").ok())
                .unwrap_or_default(),
            xai_key: std::env::var("XAI_API_KEY").unwrap_or_default(),
            phonon: Arc::new(Mutex::new(None)),
        }
    }

    async fn generate_speech(&self, args: &TtsArgs) -> ToolResult {
        if args.text.trim().is_empty() {
            return ToolResult::error("text_to_speech", "Text is required");
        }

        let provider = args.provider.as_str();

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
            "phonon" => self.phonon_local(args).await,
            _ => ToolResult::error(
                "text_to_speech",
                format!(
                    "Unknown provider: {}. Available: {}",
                    args.provider,
                    BUILTIN_PROVIDERS.join(", ")
                ),
            ),
        }
    }

    // =========================================================================
    // Edge TTS (free, no API key — the `edge-tts` CLI)
    // =========================================================================
    /// Synthesize through Microsoft Edge's TTS service via the `edge-tts`
    /// CLI (the `edge-tts` PyPI package). Keyless, but it is NOT local: the
    /// binary talks to Microsoft's endpoint, so it needs network access even
    /// though it needs no key.
    ///
    /// This was a stub until iter-2538: it returned `ToolResult::success` with
    /// a message telling the operator to install the package, and NO audio —
    /// a caller that asked for `edge` and got `success: true` had nothing to
    /// play. Measured before the fix: `audio_b64_len=0` with `success=true`,
    /// while the ten sibling providers all returned bytes. A tool that reports
    /// success without producing its product is worse than one that errors,
    /// because the agent moves on and the operator finds out at playback.
    ///
    /// The CLI shape is the same subprocess contract `piper_local` and
    /// `kittentts_local` use: write a temp file, read the bytes back, delete.
    /// `--no-progress` keeps the progress bar off stderr; a non-zero exit OR a
    /// zero-length file is an error, because the package exits 0 on empty
    /// text and leaves an empty file behind.
    async fn edge_tts(&self, args: &TtsArgs) -> ToolResult {
        let voice = resolve_voice(&args.voice, "en-US-AriaNeural");

        let Some((binary, prefix_args)) = edge_tts_command() else {
            return ToolResult::error(
                "text_to_speech",
                "Edge TTS not installed. Install it with: pip install edge-tts",
            );
        };

        let temp_dir = std::env::temp_dir();
        let output_path = temp_dir.join(format!("edge_tts_output_{}.mp3", std::process::id()));

        // The text goes in as an ARGUMENT (`--text`), not through a shell: a
        // subprocess is not a shell, so a quote in the text cannot escape the
        // argument, and no caller-supplied string is ever interpolated into
        // something a shell would parse.
        let mut command = Command::new(binary);
        command
            .args(prefix_args)
            .arg("--voice")
            .arg(voice)
            .arg("--text")
            .arg(&args.text)
            .arg("--write-media")
            .arg(&output_path)
            .arg("--no-progress");
        let result = command.output();

        match result {
            Ok(output) if output.status.success() => {
                // A zero exit is NOT proof of audio: the package exits 0 on
                // empty input and leaves a zero-byte file. Only the bytes are.
                match std::fs::metadata(&output_path) {
                    Ok(meta) if meta.len() > 0 => match std::fs::read(&output_path) {
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
                                    "format": "mp3",
                                    "provider": "edge",
                                    "voice": voice
                                }),
                            )
                        }
                        Err(e) => {
                            let _ = std::fs::remove_file(&output_path);
                            ToolResult::error(
                                "text_to_speech",
                                format!("Failed to read output: {}", e),
                            )
                        }
                    },
                    // Reached when the file is absent OR empty — including the
                    // zero-byte file the CLI leaves on empty input.
                    _ => {
                        let _ = std::fs::remove_file(&output_path);
                        ToolResult::error(
                            "text_to_speech",
                            "Edge TTS produced no audio (the edge-tts CLI exited 0 but wrote no media)",
                        )
                    }
                }
            }
            Ok(output) => {
                let _ = std::fs::remove_file(&output_path);
                let stderr = String::from_utf8_lossy(&output.stderr);
                ToolResult::error(
                    "text_to_speech",
                    format!("Edge TTS failed: {}", stderr.trim()),
                )
            }
            Err(e) => {
                let _ = std::fs::remove_file(&output_path);
                ToolResult::error("text_to_speech", format!("Failed to run edge-tts: {}", e))
            }
        }
    }

    // =========================================================================
    // ElevenLabs TTS
    // =========================================================================
    async fn elevenlabs_tts(&self, args: &TtsArgs) -> ToolResult {
        if self.elevenlabs_key.is_empty() {
            return ToolResult::error("text_to_speech", "ELEVENLABS_API_KEY not set");
        }

        let model = args.model.as_deref().unwrap_or("eleven_multilingual_v2");
        let voice_id = resolve_voice(&args.voice, "pNInz6obpgDQGcFmaJgB");

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
        let voice = resolve_voice(&args.voice, "alloy");

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

        // The caller may name a model; the default is the current MiniMax
        // generation. The old pin (`speech-01`, bare) is not in the vendor's
        // current enum at all — only `speech-01-hd` / `speech-01-turbo` are —
        // so an unupdated config would fail against today's API.
        let model = args
            .model
            .as_deref()
            .filter(|m| !m.is_empty())
            .unwrap_or(MINIMAX_DEFAULT_MODEL);

        let voice_id = resolve_voice(&args.voice, MINIMAX_DEFAULT_VOICE);

        let response = match self
            .client
            .post(MINIMAX_SPEECH_URL)
            .header("Authorization", format!("Bearer {}", self.minimax_key))
            .header("Content-Type", "application/json")
            .json(&json!({
                "model": model,
                "text": args.text,
                "stream": false,
                "language_boost": "auto",
                "output_format": "hex",
                "voice_setting": {
                    "voice_id": voice_id,
                    "speed": 1.0,
                    "vol": 1.0,
                    "pitch": 0
                },
                "audio_setting": {
                    "sample_rate": 32000,
                    "bitrate": 128000,
                    "format": "mp3",
                    "channel": 1
                }
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

        // The transport succeeded, but that is not proof of audio: t2a_v2
        // answers 200 with `base_resp.status_code != 0` for a refused
        // request, and `data` may be null. Both used to fall through to a
        // success carrying an empty audio string.
        if let Some(code) = result
            .get("base_resp")
            .and_then(|b| b.get("status_code"))
            .and_then(Value::as_i64)
            && code != 0
        {
            let msg = result
                .get("base_resp")
                .and_then(|b| b.get("status_msg"))
                .and_then(Value::as_str)
                .unwrap_or("no message");
            return ToolResult::error("text_to_speech", format!("MiniMax API error {code}: {msg}"));
        }

        // With `output_format: "hex"` the payload is hex-encoded, not base64.
        // Returning the hex as if it were base64 produced "audio" no client
        // could decode.
        let audio_hex = result
            .get("data")
            .and_then(|d| d.get("audio"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if audio_hex.is_empty() {
            return ToolResult::error(
                "text_to_speech",
                "MiniMax returned no audio data in data.audio",
            );
        }
        let audio_bytes = match hex_decode(audio_hex) {
            Some(bytes) => bytes,
            None => {
                return ToolResult::error("text_to_speech", "MiniMax data.audio was not valid hex");
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
                "provider": "minimax",
                "voice": voice_id,
                "model": model
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

        let voice_id = resolve_voice(&args.voice, "c69964a6-ab8b-4f8a-9465-ec0925096ec8");

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
            // Name both variables: the operator may have set either one, and
            // "GEMINI_API_KEY not set" when they set GOOGLE_API_KEY (which is
            // what `operant auth add google` writes) reads as a broken install
            // rather than a key-name mismatch.
            return ToolResult::error(
                "text_to_speech",
                "GEMINI_API_KEY (or GOOGLE_API_KEY) not set",
            );
        }

        let voice = resolve_voice(&args.voice, GEMINI_DEFAULT_VOICE);

        // `gemini-2.5-flash-preview-tts` is the previous TTS generation and
        // carries a published shutdown window, so the default is the current
        // one. A caller may still name a model (including the 2.5 preview)
        // while it is served.
        let model = args
            .model
            .as_deref()
            .filter(|m| !m.is_empty())
            .unwrap_or(GEMINI_DEFAULT_MODEL);

        // The key goes in the query parameter only. Sending BOTH an
        // `Authorization: Bearer` header and `?key=` is not a stricter check —
        // it is a rejected request shape, and it put the credential in a
        // second place for no benefit.
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"
        );

        let response = match self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .query(&[("key", &self.gemini_key)])
            .json(&json!({
                "contents": [{
                    "parts": [{
                        "text": args.text
                    }]
                }],
                "generationConfig": {
                    "responseModalities": ["AUDIO"],
                    "speechConfig": {
                        "voiceConfig": {
                            "prebuiltVoiceConfig": {
                                "voiceName": voice
                            }
                        }
                    }
                }
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

        // A 200 is not proof of audio: a blocked prompt comes back as a
        // candidate with `finishReason` and no `inlineData`. Reporting success
        // with an empty string is the bug this check exists to prevent.
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
            .and_then(Value::as_str)
            .unwrap_or("");
        if audio_base64.is_empty() {
            let reason = result
                .get("candidates")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|c| c.get("finishReason"))
                .and_then(Value::as_str)
                .unwrap_or("no candidate");
            return ToolResult::error(
                "text_to_speech",
                format!("Gemini returned no audio (finishReason: {reason})"),
            );
        }

        ToolResult::success(
            "text_to_speech",
            json!({
                "success": true,
                "audio": audio_base64,
                "format": "mp3",
                "provider": "gemini",
                "voice": voice,
                "model": model
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

        let voice_id = resolve_voice(&args.voice, "eve");

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
        let voice = resolve_voice(&args.voice, "neutral");
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
        let voice = resolve_voice(&args.voice, "Jasper");
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
        let voice = resolve_voice(&args.voice, "en_US-lessac-medium");

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

    /// Phonon (gradium/phonon-31m-en) — the local, offline default voice.
    ///
    /// The engine is loaded lazily on first call: `phonon::load_or_download`
    /// fetches a pinned GGUF checkpoint into `~/.operant/cache/phonon/` when
    /// it is missing and returns a warm `Synth` otherwise. `ptts` surfaces
    /// errors as `Result`, so unlike the old `ort` binding there is no
    /// panic-on-load hazard to catch here.
    async fn phonon_local(&self, args: &TtsArgs) -> ToolResult {
        let voice = resolve_voice(&args.voice, "default");

        let mut engine_lock = self.phonon.lock().await;
        if engine_lock.is_none() {
            match phonon::load_or_download().await {
                Ok(engine) => *engine_lock = Some(engine),
                Err(e) => {
                    return ToolResult::error(
                        "text_to_speech",
                        format!("Failed to initialize Phonon engine: {e}"),
                    );
                }
            }
        }

        // The `Some` above is the only way this block is reachable without an
        // early return, so the `None` arm is unreachable in practice. It is
        // still handled rather than `expect`ed: a panic in the TUI or gateway
        // would take the whole session down for a tool failure.
        let Some(engine) = engine_lock.as_ref() else {
            return ToolResult::error("text_to_speech", "Phonon engine failed to initialize");
        };

        match engine.synthesize(&args.text, Some(voice)) {
            Ok(samples) => match phonon::write_wav(&samples, engine.sample_rate()) {
                Ok(wav_bytes) => {
                    let audio_base64 = base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        &wav_bytes,
                    );
                    ToolResult::success(
                        "text_to_speech",
                        json!({
                            "success": true,
                            "audio": audio_base64,
                            "format": "wav",
                            "provider": "phonon",
                            "voice": voice
                        }),
                    )
                }
                Err(e) => ToolResult::error(
                    "text_to_speech",
                    format!("Failed to encode Phonon audio: {e}"),
                ),
            },
            Err(e) => ToolResult::error("text_to_speech", format!("Phonon synthesis failed: {e}")),
        }
    }
}

#[async_trait]
impl OperantTool for TtsTool {
    fn name(&self) -> &str {
        "text_to_speech"
    }

    fn description(&self) -> &str {
        "Convert text to speech using AI voice providers (Phonon [local, default], Edge [keyless CLI], ElevenLabs, OpenAI, MiniMax, Mistral, Gemini, xAI, NeuTTS, KittenTTS, Piper). Omit voice for the provider's own default."
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
            // A config that still names the retired Kokoro engine (the default
            // until iter-2513) must not turn into a hard tool error: the
            // operator's file parsed yesterday and has to keep parsing today.
            // Unknown names degrade to the default the same way the memory
            // provider degrades an unknown name to memory-wire — silently, so
            // a stale config is a working voice rather than a broken turn.
            args.provider = resolve_provider_name(&cfg.tts.provider);
        }
        if args.voice == default_voice()
            && let Some(ref v) = cfg.tts.voice
            && !v.is_empty()
        {
            args.voice = v.clone();
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
        assert_eq!(default_provider(), "phonon");
    }

    #[test]
    fn test_default_voice() {
        assert_eq!(default_voice(), "default");
    }

    /// The schema default `voice = "default"` must not be sent to a provider
    /// that has no voice by that name.
    ///
    /// `TtsArgs::voice` defaults to `"default"` (the SCHEMA sentinel), and
    /// every provider used to pass it through verbatim whenever it was not
    /// EMPTY. Only Phonon has a voice named `default`; `edge-tts` rejects it
    /// outright (`ValueError: Invalid voice 'default'.`), and OpenAI /
    /// ElevenLabs / Mistral / xAI reject any unknown voice id. Since an
    /// omitted `voice` is the common case, every one of the ten non-Phonon
    /// providers failed on the default call. Both spellings of "not chosen"
    /// must resolve to the provider's own default.
    #[test]
    fn the_schema_default_voice_resolves_to_each_providers_own_default() {
        assert_eq!(resolve_voice("", "alloy"), "alloy");
        assert_eq!(resolve_voice("default", "alloy"), "alloy");
        assert_eq!(
            resolve_voice("default", "en-US-AriaNeural"),
            "en-US-AriaNeural"
        );
        assert_eq!(resolve_voice("default", "default"), "default");
        // A chosen voice is never rewritten.
        assert_eq!(resolve_voice("nova", "alloy"), "nova");
        assert_eq!(
            resolve_voice("en-GB-SoniaNeural", "en-US-AriaNeural"),
            "en-GB-SoniaNeural"
        );
    }

    /// The invariant the ten cloud/local providers share and `edge` broke:
    /// success carries decodable audio. Exercised here over every provider
    /// this host can reach without credentials, so a provider that regresses
    /// into a stub fails in the unit suite rather than in the operator's ear.
    ///
    /// The provider list is derived from `BUILTIN_PROVIDERS`, not written out
    /// here (iter-2590). It used to be a second hardcoded copy of the same
    /// eleven names, so a twelfth provider added to the dispatch would have
    /// been silently ABSENT from this test -- and `assert_success_carries_audio`
    /// passes vacuously for a name it never sees. That is the same
    /// drift-a-hardcoded-list shape as iter-2586, in the test that guards the
    /// stub regression itself.
    #[tokio::test]
    async fn no_builtin_provider_reports_success_without_audio() {
        let tool = TtsTool::new();
        // Guard the derivation itself: an empty or truncated list would make
        // the loop below pass without exercising anything.
        assert!(
            BUILTIN_PROVIDERS.len() >= 11,
            "BUILTIN_PROVIDERS shrank to {} entries -- this test would cover almost nothing",
            BUILTIN_PROVIDERS.len()
        );
        for provider in BUILTIN_PROVIDERS {
            let result = tool
                .execute(
                    json!({"text": "Audit probe.", "provider": provider}),
                    ToolContext::default(),
                )
                .await;
            // An unreachable provider must fail. It must never succeed while
            // carrying no audio — the exact defect this iteration fixed.
            assert_success_carries_audio(&result);
        }
    }

    /// The two providers whose vendor APIs moved under us. Both were pinned to
    /// a model id / endpoint that today's API rejects, so the provider was
    /// configured correctly and still could not synthesize. A pin that drifts
    /// is invisible until someone calls it, which is why the values are
    /// asserted here rather than left to a comment.
    #[test]
    fn the_minimax_and_gemini_pins_match_the_current_vendor_apis() {
        assert_eq!(MINIMAX_SPEECH_URL, "https://api.minimax.io/v1/t2a_v2");
        // Bare `speech-01` is not in the vendor enum any more.
        assert_eq!(MINIMAX_DEFAULT_MODEL, "speech-2.8-hd");
        // A voice the current API ships; the old default belonged to the
        // retired generation.
        assert_eq!(MINIMAX_DEFAULT_VOICE, "English_expressive_narrator");

        assert_eq!(GEMINI_DEFAULT_MODEL, "gemini-3.8-flash-tts");
        assert_ne!(GEMINI_DEFAULT_MODEL, "gemini-2.5-flash-preview-tts");
    }

    /// `t2a_v2` returns hex under `output_format: "hex"`. Handing that back
    /// verbatim as if it were base64 is what the old code did, and no client
    /// could decode the result while the tool reported success.
    #[test]
    fn minimax_hex_payload_is_decoded_not_passed_through() {
        assert_eq!(hex_decode("48656c6c6f"), Some(b"Hello".to_vec()));
        // The `0x` prefix some payloads carry is not part of the bytes.
        assert_eq!(hex_decode("0x48656c6c6f"), Some(b"Hello".to_vec()));
        // Empty is a missing payload, not empty audio.
        assert_eq!(hex_decode(""), Some(Vec::new()));
        // Malformed input is refused rather than silently truncated.
        assert_eq!(hex_decode("abc"), None);
        assert_eq!(hex_decode("zz"), None);
    }

    #[test]
    fn an_unknown_provider_name_degrades_to_the_default_instead_of_erroring() {
        // The regression this pins: iter-2513 retired `kokoro`, and an
        // "Unknown provider" on every request — a config that parsed
        // yesterday failing today.
        assert_eq!(resolve_provider_name("kokoro"), default_provider());
        assert_eq!(resolve_provider_name(""), default_provider());
        // An unknown name (a typo, or a retired/never-installed plugin)
        // degrades the same way rather than erroring — the doc/CHANGELOG promise.
        assert_eq!(resolve_provider_name("openai-tts"), default_provider());
        assert_eq!(resolve_provider_name("nonexistent"), default_provider());
        // A name this build genuinely has is left alone.
        assert_eq!(resolve_provider_name("edge"), "edge");
        assert_eq!(resolve_provider_name("phonon"), "phonon");
    }

    #[test]
    fn the_builtin_provider_list_matches_the_dispatch_arms() {
        // BUILTIN_PROVIDERS feeds the unknown-provider message; a name in it
        // with no dispatch arm would be advertised and then refused.
        for name in BUILTIN_PROVIDERS {
            assert!(
                !name.is_empty(),
                "empty name in BUILTIN_PROVIDERS would render as a bare comma"
            );
        }
        assert!(BUILTIN_PROVIDERS.contains(&default_provider().as_str()));
        assert!(BUILTIN_PROVIDERS.contains(&"phonon"));

        // The name says this test compares the list against the DISPATCH ARMS,
        // but until iter-2586 it only checked that the list was non-empty and
        // contained two hardcoded names. A provider added to BUILTIN_PROVIDERS
        // with no arm (or an arm whose name was renamed) was advertised in the
        // "unknown provider" message, selectable in the setup wizard, and then
        // REFUSED at dispatch -- the exact failure the comment above claims to
        // prevent. Compare the two for real: the set of advertised names must
        // equal the set the tool can actually reach.
        let advertised: std::collections::BTreeSet<&str> =
            BUILTIN_PROVIDERS.iter().copied().collect();
        let reachable: std::collections::BTreeSet<&str> = [
            "edge",
            "elevenlabs",
            "openai",
            "minimax",
            "mistral",
            "gemini",
            "xai",
            "neutts",
            "kittentts",
            "piper",
            "phonon",
        ]
        .into_iter()
        .collect();
        assert_eq!(
            advertised, reachable,
            "BUILTIN_PROVIDERS and the dispatch arms disagree: advertised {:?} vs reachable {:?}",
            advertised, reachable
        );
        // A duplicate would make the unknown-provider message repeat a name.
        assert_eq!(
            BUILTIN_PROVIDERS.len(),
            advertised.len(),
            "BUILTIN_PROVIDERS has a duplicate entry"
        );
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

    /// Model-backed check: the Phonon checkpoint downloads into the cache and
    /// the tool returns audible 24 kHz speech. Run with
    /// `cargo test -p operant-core phonon_speaks -- --ignored`.
    ///
    /// This is the live counterpart to the unit tests above: those prove the
    /// tool's wiring and schema without a model, this one proves the model
    /// actually loads and speaks.
    #[tokio::test]
    #[ignore = "needs the Phonon checkpoint in ~/.operant/cache/phonon (downloads ~65 MB)"]
    async fn phonon_speaks_through_the_tool_with_the_cached_model() {
        let tool = TtsTool::new();
        let result = tool
            .execute(
                json!({"text": "Operant audio check.", "provider": "phonon"}),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "{:?}", result.error);
        let content: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        let wav = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            content["audio"].as_str().unwrap(),
        )
        .unwrap();
        let mut reader = hound::WavReader::new(std::io::Cursor::new(wav)).unwrap();
        let spec = reader.spec();
        let samples: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
        let seconds = samples.len() as f32 / spec.sample_rate as f32;
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        eprintln!(
            "phonon live check: {seconds:.2}s at {} Hz, peak {peak}",
            spec.sample_rate
        );
        assert_eq!(spec.sample_rate, 24_000);
        assert!((0.8..8.0).contains(&seconds), "duration {seconds}s");
        assert!(peak > 1_000, "silent output (peak {peak})");
    }

    // =====================================================================
    // edge: the stub that reported success without audio (iter-2538)
    // =====================================================================

    /// The invariant every provider must hold, and the one `edge` broke until
    /// iter-2538: a call that reports SUCCESS carries decodable audio.
    ///
    /// This is the assertion to keep if the edge body is ever rewritten. It
    /// is a property of the RESULT, not of the implementation, so it does not
    /// care how `edge` is dispatched.
    fn assert_success_carries_audio(result: &ToolResult) {
        // An ERROR result has empty content by construction
        // (`ToolResult::error`), so the audio is read with `.ok()` rather than
        // by unwrapping the parse.
        let audio: String = serde_json::from_str::<serde_json::Value>(&result.content)
            .ok()
            .as_ref()
            .and_then(|v| v.get("audio"))
            .and_then(|a| a.as_str())
            .unwrap_or("")
            .to_string();
        if result.success {
            assert!(
                !audio.is_empty(),
                "a successful text_to_speech must carry audio; content={:?} error={:?}",
                result.content,
                result.error
            );
            assert!(
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &audio).is_ok(),
                "the audio a successful text_to_speech reports must be valid base64"
            );
        }
    }

    /// THE regression guard for the defect this iteration fixes.
    ///
    /// Before the fix, `provider = "edge"` returned `success: true` with
    /// `audio` absent and `audio_b64_len = 0` — measured on the deployed
    /// binary, against the ten sibling providers which all returned bytes.
    /// A provider that cannot be reached must now FAIL, not claim success.
    ///
    /// The test is written to hold whether or not `edge-tts` is installed:
    /// if it is installed the call synthesizes (and must carry audio); if it
    /// is not, the call must error. Either way the forbidden state — success
    /// with no audio — fails.
    #[tokio::test]
    async fn an_edge_call_cannot_report_success_without_audio() {
        let tool = TtsTool::new();
        let result = tool
            .execute(
                json!({"text": "Audit probe, one two three.", "provider": "edge"}),
                ToolContext::default(),
            )
            .await;

        assert_success_carries_audio(&result);
        if !result.success {
            // The failure must say WHY, not be an empty error.
            let error = result.error.clone().unwrap_or_default();
            assert!(
                !error.trim().is_empty(),
                "a failed edge call must explain itself"
            );
        }
    }

    /// A missing `edge-tts` must produce the install hint, not a subprocess
    /// error. PATH is emptied so this holds on any host: if the binary IS
    /// installed here, this test would otherwise prove nothing.
    ///
    /// `serial_test` is required: `PATH` is process-global and every other
    /// test in this binary that spawns a tool depends on it.
    #[tokio::test]
    #[serial_test::serial]
    async fn edge_without_the_cli_errors_with_the_install_hint() {
        // The crate-wide environment lock, NOT only `#[serial_test::serial]`.
        // They are two different locks and they do not exclude each other: a
        // `serial` test excludes other `serial` tests and nothing else, so the
        // 60 tests that hold `test_env_lock_blocking()` ran straight through this
        // empty-`PATH` window. MEASURED: `checkpoint_tool`'s git-backed tests
        // then fail `mgr.ensure_checkpoint(...)` because `Command::new("git")`
        // cannot resolve the binary -- 11 tests, only under parallelism, and
        // the failure reads as a broken checkpoint feature rather than as a
        // missing `PATH`. Any test that mutates the environment holds this lock.
        let _env_guard = crate::platform::test_env_lock();
        let tool = TtsTool::new();
        let Some(original) = std::env::var_os("PATH") else {
            return;
        };
        // SAFETY: the restore below is unconditional on every path after the
        // `let-else` above, and `serial_test` keeps other tests out of `PATH`.
        unsafe { std::env::set_var("PATH", "") };
        let result = tool
            .execute(
                json!({"text": "Audit probe.", "provider": "edge"}),
                ToolContext::default(),
            )
            .await;
        unsafe { std::env::set_var("PATH", original) };

        assert!(
            !result.success,
            "a missing CLI is not a successful synthesis"
        );
        let error = result.error.unwrap_or_default();
        assert!(
            error.contains("edge-tts"),
            "the error must name the missing package: {error}"
        );
        assert!(
            error.contains("pip install"),
            "the error must say how to install it: {error}"
        );
    }

    /// The live counterpart, for a host that HAS the package: prove the bytes
    /// are real MP3, not an empty string. Run with
    /// `cargo test -p operant-core edge_speaks -- --ignored`.
    #[tokio::test]
    #[ignore = "needs the edge-tts package (pip install edge-tts) and network access"]
    async fn edge_speaks_through_the_tool() {
        let tool = TtsTool::new();
        let result = tool
            .execute(
                json!({"text": "Operant audio check.", "provider": "edge"}),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "{:?}", result.error);
        assert_success_carries_audio(&result);
        let content: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        let audio = content["audio"].as_str().unwrap();
        // A real MP3 for a five-word sentence is kilobytes, not bytes.
        assert!(
            audio.len() > 1_000,
            "edge returned only {} base64 chars",
            audio.len()
        );
        let mp3 =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, audio).unwrap();
        // An MPEG frame sync is the cheapest proof it is audio: 0xFF then
        // 0xE0-masked bits. An empty or HTML error page would not match.
        assert!(
            mp3.len() > 2 && mp3[0] == 0xFF && (mp3[1] & 0xE0) == 0xE0,
            "edge output is not an MPEG frame: {:?}",
            &mp3[..mp3.len().min(4)]
        );
    }

    /// `operant auth add google` writes `GOOGLE_API_KEY`; this provider must see it.
    ///
    /// `cmd_auth`'s provider table names `GOOGLE_API_KEY` for `google`
    /// (cmd_auth.rs), while the TTS provider read only `GEMINI_API_KEY`. An
    /// operator who added a Google key through the tool's OWN auth command was
    /// told "GEMINI_API_KEY not set" for a key they had already configured, and
    /// nothing in core ever consulted the other name.
    ///
    /// Both names are accepted now. This test is the whole guard: it can only
    /// pass if `TtsTool::new` reads the fallback, because the error message
    /// names the variable that is missing.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_google_api_key_configured_by_auth_is_visible_to_gemini_tts() {
        // Crate-wide environment lock, for the same reason as the test above:
        // `#[serial_test::serial]` and `test_env_lock()` are different locks.
        let _env_guard = crate::platform::test_env_lock();
        // BOTH vars are captured, whatever they are. An earlier version of this
        // test returned early when `GEMINI_API_KEY` was unset -- which is the
        // normal case on a machine that configured Google through
        // `operant auth` -- so it tested nothing and passed with the fix
        // reverted. A test that cannot fail is not a test.
        let original_gemini = std::env::var_os("GEMINI_API_KEY");
        let original_google = std::env::var_os("GOOGLE_API_KEY");
        // SAFETY: serialised by `#[serial_test::serial]`; both vars are
        // restored below on every path.
        unsafe { std::env::remove_var("GEMINI_API_KEY") };
        unsafe { std::env::set_var("GOOGLE_API_KEY", "probe-google-key") };

        let tool = TtsTool::new();
        let result = tool
            .execute(
                json!({"text": "Audit probe.", "provider": "gemini"}),
                ToolContext::default(),
            )
            .await;

        unsafe { std::env::remove_var("GOOGLE_API_KEY") };
        match original_gemini {
            Some(v) => unsafe { std::env::set_var("GEMINI_API_KEY", v) },
            None => unsafe { std::env::remove_var("GEMINI_API_KEY") },
        }
        match original_google {
            Some(v) => unsafe { std::env::set_var("GOOGLE_API_KEY", v) },
            None => unsafe { std::env::remove_var("GOOGLE_API_KEY") },
        }

        // The key IS configured, so the failure must NOT be the missing-key
        // one. A network/auth failure is fine and expected here: the point is
        // that the provider got as far as USING the credential.
        if let Some(err) = result.error {
            assert!(
                !err.contains("not set"),
                "a GOOGLE_API_KEY from `operant auth add google` must be visible                  to the gemini TTS provider, got: {err}"
            );
        }
    }
    /// The TTS dispatch is ONE tier: the eleven built-in match arms.
    /// (RT-599, iter-2609)
    ///
    /// Two further tiers were removed because they were unreachable AND
    /// contradicted AD-005 §5 ("do not add ... TTS ... registration until a
    /// concrete plugin needs one"). The danger was not the 546 dead lines --
    /// it was the NEXT editor wiring `register_command_provider` to config
    /// and shipping an unsandboxed shell surface, because that tier's
    /// `command_template` is substituted with no escaping. This guard fails
    /// the build if the seam is reintroduced, so the removal is a decision
    /// the tree enforces rather than one a comment records.
    ///
    /// Structural on purpose. A behavioural test cannot see this: with the
    /// seam wired but never populated, every dispatch behaves identically,
    /// which is exactly how it stayed hidden for five months.
    #[test]
    fn the_tts_dispatch_has_no_plugin_or_command_tier() {
        // Scan the PRODUCTION half only. `include_str!` reads this whole
        // file, which carries these identifiers as literals in the assert
        // below -- so scanning the raw string makes the guard match itself
        // and fail on a clean tree (measured: it did exactly that on the
        // first draft). Everything before the test module is production.
        let src = include_str!("tts_tool.rs");
        let production = src.split("#[cfg(test)]").next().unwrap_or_default();

        // The two tiers, by the identifiers that only they used.
        for dead in [
            "plugin_registry",
            "command_providers",
            "register_command_provider",
            "with_plugin_registry",
            "TtsPluginRegistry",
            "CommandProvider",
        ] {
            assert!(
                !production.contains(dead),
                "the dead TTS plugin/command seam is back: found `{dead}` in the production half of tts_tool.rs. AD-005 section 5 says no TTS registration without a                  concrete consumer. If a plugin genuinely needs it now, that is a new                  design with an explicit sandbox decision, not a restore."
            );
        }

        // The three modules are gone from the module tree, not merely
        // unreferenced from it. A `mod` declaration with the file deleted
        // fails to compile; a re-export pointing at a deleted module does too.
        // So this asserts what a reader would otherwise have to trust: the
        // seam is out of the crate's surface, not orphaned inside it.
        let mods = include_str!("../tools.rs");
        for module in ["tts_command_provider", "tts_provider", "tts_registry"] {
            assert!(
                !mods.contains(module),
                "`{module}` is back in the tools module tree; RT-599 removed                 it as an unreachable seam that contradicted AD-005 section 5"
            );
        }

        // And the live list is still the eleven built-ins the arms dispatch to.
        assert_eq!(
            BUILTIN_PROVIDERS.len(),
            11,
            "the built-in provider list changed; re-derive the doc claim in                  docs/library/voice.md and the wizard menu"
        );
    }
}
