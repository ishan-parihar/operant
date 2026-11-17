use async_trait::async_trait;
use reqwest::multipart::{Form, Part};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::fs;

use crate::config::AppConfig;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptionArgs {
    /// Path to audio file
    file_path: String,
    /// Provider: "groq" or "openai"
    provider: Option<String>,
    /// Model name (e.g. "whisper-large-v3-turbo" for groq, "whisper-1" for openai)
    model: Option<String>,
}

#[derive(Serialize)]
struct TranscriptionResult {
    success: bool,
    transcript: String,
    provider: String,
    model: String,
    error: Option<String>,
}

pub struct TranscriptionTool {
    config: Option<AppConfig>,
}

impl Default for TranscriptionTool {
    fn default() -> Self {
        Self::new()
    }
}

impl TranscriptionTool {
    pub fn new() -> Self {
        Self { config: None }
    }

    pub fn with_config(config: AppConfig) -> Self {
        Self {
            config: Some(config),
        }
    }
}

const SUPPORTED_FORMATS: &[&str] = &[
    ".mp3", ".mp4", ".mpeg", ".mpga", ".m4a", ".wav", ".webm", ".ogg", ".aac", ".flac",
];
const MAX_FILE_SIZE: u64 = 25 * 1024 * 1024;

const GROQ_DEFAULT_MODEL: &str = "whisper-large-v3-turbo";
const OPENAI_DEFAULT_MODEL: &str = "whisper-1";

/// Which model the request names: the per-call argument wins, then the
/// operator's `[tools.stt]` setting, then the provider default. The setting
/// exists because a provider retires model names — a hardcoded one turns into
/// a 404 with no way out short of a new build. (`groq_model` / `openai_model`
/// were accepted and advertised in `operant.example.toml` while nothing read
/// them; this is the read.)
fn resolve_model(explicit: &str, configured: Option<&str>, fallback: &str) -> String {
    if !explicit.is_empty() {
        return explicit.to_string();
    }
    match configured {
        Some(model) if !model.is_empty() => model.to_string(),
        _ => fallback.to_string(),
    }
}

fn resolve_groq_model(config: &Option<AppConfig>, explicit: &str) -> String {
    resolve_model(
        explicit,
        config.as_ref().map(|c| c.tools.stt.groq_model.as_str()),
        GROQ_DEFAULT_MODEL,
    )
}

fn resolve_openai_model(config: &Option<AppConfig>, explicit: &str) -> String {
    resolve_model(
        explicit,
        config.as_ref().map(|c| c.tools.stt.openai_model.as_str()),
        OPENAI_DEFAULT_MODEL,
    )
}

fn get_groq_api_key(config: &Option<AppConfig>) -> Option<String> {
    if let Some(cfg) = config
        && let Some(ref key) = cfg.tools.stt.groq_api_key
        && !key.is_empty()
    {
        return Some(key.clone());
    }
    std::env::var("GROQ_API_KEY").ok()
}

fn get_openai_api_key(config: &Option<AppConfig>) -> Option<String> {
    if let Some(cfg) = config
        && let Some(ref key) = cfg.tools.stt.openai_api_key
        && !key.is_empty()
    {
        return Some(key.clone());
    }
    std::env::var("OPENAI_API_KEY")
        .or_else(|_| std::env::var("VOICE_TOOLS_OPENAI_KEY"))
        .ok()
}

async fn transcribe_groq(file_path: &str, model: &str, api_key: &str) -> TranscriptionResult {
    let audio_bytes = match fs::read(file_path).await {
        Ok(b) => b,
        Err(e) => {
            return TranscriptionResult {
                success: false,
                transcript: String::new(),
                provider: "groq".to_string(),
                model: model.to_string(),
                error: Some(format!("Failed to read file: {e}")),
            };
        }
    };

    let file_name = std::path::Path::new(file_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "audio".to_string());

    // The audio bytes are captured ONLY on a successful part build. The old
    // `.unwrap_or_else(|_| Part::bytes(Vec::new()))` discarded them on a mime
    // parse failure, so the request went out with a ZERO-LENGTH file and the
    // provider answered "no audio" — a confusing error about the wrong thing,
    // while the real cause (a mime string that did not parse) went unrecorded.
    // `"audio/mpeg"` is a constant that parses, so this arm is unreachable in
    // practice; the point is that it must not be able to silently empty the
    // upload.
    let file_part = match Part::bytes(audio_bytes)
        .file_name(file_name)
        .mime_str("audio/mpeg")
    {
        Ok(part) => part,
        Err(e) => {
            return TranscriptionResult {
                success: false,
                transcript: String::new(),
                provider: "groq".to_string(),
                model: model.to_string(),
                error: Some(format!("Could not build the audio upload: {e}")),
            };
        }
    };

    let form = Form::new()
        .part("file", file_part)
        .text("model", model.to_string())
        .text("response_format", "text");

    let client = reqwest::Client::new();
    match client
        .post("https://api.groq.com/openai/v1/audio/transcriptions")
        .header("Authorization", format!("Bearer {api_key}"))
        .multipart(form)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
    {
        Ok(resp) => {
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return TranscriptionResult {
                    success: false,
                    transcript: String::new(),
                    provider: "groq".to_string(),
                    model: model.to_string(),
                    error: Some(format!("Groq API error ({}): {}", status, body)),
                };
            }
            let text = resp.text().await.unwrap_or_default();
            TranscriptionResult {
                success: !text.is_empty(),
                transcript: text.trim().to_string(),
                provider: "groq".to_string(),
                model: model.to_string(),
                error: if text.is_empty() {
                    Some("Empty response".to_string())
                } else {
                    None
                },
            }
        }
        Err(e) => TranscriptionResult {
            success: false,
            transcript: String::new(),
            provider: "groq".to_string(),
            model: model.to_string(),
            error: Some(format!("Groq request failed: {e}")),
        },
    }
}

async fn transcribe_openai(file_path: &str, model: &str, api_key: &str) -> TranscriptionResult {
    let audio_bytes = match fs::read(file_path).await {
        Ok(b) => b,
        Err(e) => {
            return TranscriptionResult {
                success: false,
                transcript: String::new(),
                provider: "openai".to_string(),
                model: model.to_string(),
                error: Some(format!("Failed to read file: {e}")),
            };
        }
    };

    let file_name = std::path::Path::new(file_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "audio".to_string());

    // See `transcribe_groq`: a mime failure must report, not empty the upload.
    let file_part = match Part::bytes(audio_bytes)
        .file_name(file_name)
        .mime_str("audio/mpeg")
    {
        Ok(part) => part,
        Err(e) => {
            return TranscriptionResult {
                success: false,
                transcript: String::new(),
                provider: "openai".to_string(),
                model: model.to_string(),
                error: Some(format!("Could not build the audio upload: {e}")),
            };
        }
    };

    let form = Form::new()
        .part("file", file_part)
        .text("model", model.to_string())
        .text("response_format", "text");

    let client = reqwest::Client::new();
    match client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .header("Authorization", format!("Bearer {api_key}"))
        .multipart(form)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
    {
        Ok(resp) => {
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return TranscriptionResult {
                    success: false,
                    transcript: String::new(),
                    provider: "openai".to_string(),
                    model: model.to_string(),
                    error: Some(format!("OpenAI API error ({}): {}", status, body)),
                };
            }
            let text = resp.text().await.unwrap_or_default();
            TranscriptionResult {
                success: !text.is_empty(),
                transcript: text.trim().to_string(),
                provider: "openai".to_string(),
                model: model.to_string(),
                error: if text.is_empty() {
                    Some("Empty response".to_string())
                } else {
                    None
                },
            }
        }
        Err(e) => TranscriptionResult {
            success: false,
            transcript: String::new(),
            provider: "openai".to_string(),
            model: model.to_string(),
            error: Some(format!("OpenAI request failed: {e}")),
        },
    }
}

#[async_trait]
impl OperantTool for TranscriptionTool {
    fn name(&self) -> &str {
        "transcribe_audio"
    }

    fn description(&self) -> &str {
        "Transcribe audio files to text using Groq Whisper or OpenAI Whisper API"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<TranscriptionArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: TranscriptionArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid arguments: {e}")),
        };

        let path = std::path::Path::new(&parsed.file_path);
        if !path.exists() {
            return ToolResult::error(self.name(), format!("File not found: {}", parsed.file_path));
        }
        if !path.is_file() {
            return ToolResult::error(self.name(), format!("Not a file: {}", parsed.file_path));
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}").to_lowercase())
            .unwrap_or_default();
        if !SUPPORTED_FORMATS.contains(&ext.as_str()) {
            return ToolResult::error(
                self.name(),
                format!(
                    "Unsupported format '{ext}'. Supported: {}",
                    SUPPORTED_FORMATS.join(", ")
                ),
            );
        }

        match fs::metadata(&parsed.file_path).await {
            Ok(meta) => {
                if meta.len() > MAX_FILE_SIZE {
                    return ToolResult::error(
                        self.name(),
                        format!(
                            "File too large: {:.1}MB (max {}MB)",
                            meta.len() as f64 / 1_048_576.0,
                            MAX_FILE_SIZE / 1_048_576
                        ),
                    );
                }
            }
            Err(e) => return ToolResult::error(self.name(), format!("Cannot access file: {e}")),
        }

        let provider = parsed.provider.as_deref().unwrap_or("groq");
        let model = parsed.model.as_deref().unwrap_or("");

        let result = match provider {
            "groq" => {
                let api_key = match get_groq_api_key(&self.config) {
                    Some(k) => k,
                    None => {
                        return ToolResult::error(
                            self.name(),
                            "GROQ_API_KEY not set. Set groqApiKey in tool settings or GROQ_API_KEY env var",
                        );
                    }
                };
                let model = resolve_groq_model(&self.config, model);
                transcribe_groq(&parsed.file_path, &model, &api_key).await
            }
            "openai" => {
                let api_key = match get_openai_api_key(&self.config) {
                    Some(k) => k,
                    None => return ToolResult::error(self.name(), "OPENAI_API_KEY not set"),
                };
                let model = resolve_openai_model(&self.config, model);
                transcribe_openai(&parsed.file_path, &model, &api_key).await
            }
            other => {
                return ToolResult::error(
                    self.name(),
                    format!("Unknown provider: '{other}'. Use: groq, openai"),
                );
            }
        };

        transcription_envelope(self.name(), result)
    }
}

/// The tool envelope for a finished provider call: the PROVIDER's own verdict
/// decides it, not a constant.
///
/// `transcribe_groq` / `transcribe_openai` report an API error, an empty
/// response and a network fault IN BAND — a `TranscriptionResult` with
/// `success: false` plus an `error` string. This call site used to wrap every
/// outcome in `ToolResult::success`, so the envelope read `success: true` with
/// `error: None` while the payload said the opposite.
///
/// That is the edge_tts shape (iter-2547), and it blinded every guard that
/// reads the envelope rather than the payload:
///
/// - `consecutive_failed_iters` in the agent loop never incremented, so a model
///   retrying a transcription that fails every time never reached the
///   degenerate-loop breaker;
/// - the tool guardrail (`agent/stream.rs`) scored the batch as successful
///   progress, so a narration claiming progress was believed;
/// - the observer recorded `success: true`;
/// - no `AgentEvent::ToolError` reached the transcript.
///
/// A free function rather than an inline branch so the rule is testable without
/// a network call: the provider helpers are private and their failure paths all
/// require I/O.
fn transcription_envelope(tool_call_id: &str, result: TranscriptionResult) -> ToolResult {
    if result.success {
        return ToolResult::success(tool_call_id, result);
    }
    // `ToolResult::error` sets `error` and empties `content`, so the
    // provider's own text (status code + response body) has to be carried into
    // `error` — it is the only account of WHY it failed, and a model cannot
    // retry intelligently against a bare "Error".
    let reason = result
        .error
        .unwrap_or_else(|| "transcription failed with no error text".to_string());
    ToolResult::error(tool_call_id, reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_transcription_schema() {
        let tool = TranscriptionTool::new();
        let schema = tool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "transcribe_audio");
    }

    #[tokio::test]
    async fn test_transcription_invalid_args() {
        let tool = TranscriptionTool::new();
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success);
    }

    /// `[tools.stt] groq_model` / `openai_model` are advertised settings;
    /// before this test they were parsed and ignored, so a retired model name
    /// could only be worked around in a new build.
    #[test]
    fn a_configured_stt_model_overrides_the_builtin_default() {
        let mut config = AppConfig::default();
        config.tools.stt.groq_model = "whisper-large-v3".to_string();
        config.tools.stt.openai_model = "gpt-4o-transcribe".to_string();
        let config = Some(config);

        assert_eq!(resolve_groq_model(&config, ""), "whisper-large-v3");
        assert_eq!(resolve_openai_model(&config, ""), "gpt-4o-transcribe");
        // A per-call model still wins, and an unreadable config falls back.
        assert_eq!(
            resolve_groq_model(&config, "whisper-turbo-x"),
            "whisper-turbo-x"
        );
        assert_eq!(resolve_groq_model(&None, ""), GROQ_DEFAULT_MODEL);
        assert_eq!(resolve_openai_model(&None, ""), OPENAI_DEFAULT_MODEL);
    }

    #[test]
    fn an_empty_configured_stt_model_falls_back_to_the_default() {
        let mut config = AppConfig::default();
        config.tools.stt.groq_model = String::new();
        let config = Some(config);
        assert_eq!(resolve_groq_model(&config, ""), GROQ_DEFAULT_MODEL);
    }

    fn a_failed_result(success: bool, error: Option<&str>) -> TranscriptionResult {
        TranscriptionResult {
            success,
            transcript: String::new(),
            provider: "groq".to_string(),
            model: "whisper-large-v3-turbo".to_string(),
            error: error.map(str::to_string),
        }
    }

    /// THE CONTRACT: a provider failure must not arrive at the caller as a
    /// success.
    ///
    /// Every pre-fix outcome — an API error, an empty response body, a network
    /// fault — produced `ToolResult { success: true, error: None }` while the
    /// payload it wrapped said `success: false`.
    ///
    /// That blinded every guard that reads the envelope rather than the
    /// payload: `consecutive_failed_iters` in the agent loop never incremented,
    /// so a model retrying a permanently-failing transcription never reached
    /// the degenerate-loop breaker; the tool guardrail scored the batch as
    /// successful progress; the observer recorded `success: true`; and no
    /// `ToolError` reached the transcript.
    #[test]
    fn a_failed_transcription_is_not_reported_as_a_success() {
        let envelope = transcription_envelope(
            "call-1",
            a_failed_result(false, Some("Groq API error (429): rate limited")),
        );
        assert!(
            !envelope.success,
            "a transcription that produced no text must not arrive as a success \
             (the edge_tts shape, iter-2547)"
        );
        // The error field is what the agent loop, the observer and the
        // transcript read; it must carry the provider's reason, not be empty.
        assert_eq!(
            envelope.error.as_deref(),
            Some("Groq API error (429): rate limited"),
            "the provider's own reason must reach the caller"
        );
    }

    /// A failure with no error text still fails, and still says something:
    /// `ToolResult::error` with an empty string would leave the model with
    /// nothing to act on.
    #[test]
    fn a_failed_transcription_without_error_text_still_says_so() {
        let envelope = transcription_envelope("call-2", a_failed_result(false, None));
        assert!(!envelope.success, "no text was produced, so it failed");
        let reason = envelope.error.unwrap_or_default();
        assert!(
            !reason.is_empty(),
            "a failure must not carry an empty reason"
        );
    }

    /// The SUCCESS arm is unaffected: a result whose provider flag is true
    /// still travels as a success. Without this, "always return an error"
    /// would satisfy the test above.
    #[test]
    fn a_successful_transcription_still_reports_success() {
        let mut result = a_failed_result(true, None);
        result.transcript = "hello".to_string();
        let envelope = transcription_envelope("call-3", result);
        assert!(
            envelope.success,
            "a successful transcription must still report success"
        );
        assert!(envelope.error.is_none(), "a success carries no error");
    }

    /// SOURCE contract over the call site: `execute` must route through the
    /// shared envelope rule. Without this, the helper could be correct while
    /// the production path kept the unconditional `ToolResult::success` — the
    /// iter-2528 lesson (a helper test cannot see the call site).
    #[test]
    fn execute_routes_through_the_shared_envelope() {
        let src = include_str!("transcription_tool.rs");
        let call = format!("transcription_{}(self.name(), result)", "envelope");
        assert_eq!(
            src.matches(&call).count(),
            1,
            "`execute` must build its result through the shared envelope rule, \
             exactly once: {call}"
        );
        // And the defective shape must not survive anywhere in the file.
        let unconditional = format!("ToolResult::{}(self.name(), result)", "success");
        assert_eq!(
            src.matches(&unconditional).count(),
            0,
            "an unconditional success envelope is the edge_tts shape"
        );
    }

    /// A part-build failure must REPORT, not silently empty the upload.
    ///
    /// The old code was `.unwrap_or_else(|_| Part::bytes(Vec::new()))`: a mime
    /// parse failure produced a ZERO-LENGTH part, so the provider received an
    /// empty file and answered "no audio" — an error about the wrong thing,
    /// while the real cause (a mime string that did not parse) went unrecorded.
    /// The bytes were consumed by `Part::bytes` before the fallible call, so the
    /// fallback could not even keep the audio.
    ///
    /// SOURCE contract over the seam (the repo's own shape): the defective
    /// pattern must be gone from BOTH provider helpers, and each must build its
    /// part through a fallible `match` that returns on `Err`. The needles are
    /// concatenation-built so this test's own source cannot satisfy them.
    #[test]
    fn a_mime_failure_is_reported_rather_than_emptying_the_upload() {
        let src = include_str!("transcription_tool.rs");
        // The defective shape: an empty part as the error fallback.
        let discarded = format!("Part::bytes(Vec::{})", "new");
        assert_eq!(
            src.matches(&discarded).count(),
            0,
            "a mime failure must not discard the audio and upload an empty part"
        );
        // Both helpers must build the part through the fallible path.
        let call = format!("Part::bytes({}", "audio_bytes");
        assert_eq!(
            src.matches(&call).count(),
            2,
            "both provider helpers must build their part from the real bytes: {call}"
        );
        // And each failure arm must say what went wrong.
        let reported = format!("Could not build the audio {}", "upload");
        assert_eq!(
            src.matches(&reported).count(),
            2,
            "each provider must report a part-build failure with its cause"
        );
    }

    #[tokio::test]
    async fn test_transcription_file_not_found() {
        let tool = TranscriptionTool::new();
        let result = tool
            .execute(
                json!({"filePath": "/nonexistent/file.mp3"}),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
    }
}
