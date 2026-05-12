use async_trait::async_trait;
use reqwest::multipart::{Form, Part};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::fs;

use crate::config::AppConfig;
use crate::tools::{HermesTool, ToolContext, ToolResult};
use crate::schema::ToolSchema;

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

impl TranscriptionTool {
    pub fn new() -> Self {
        Self { config: None }
    }

    pub fn with_config(config: AppConfig) -> Self {
        Self { config: Some(config) }
    }
}

const SUPPORTED_FORMATS: &[&str] = &[".mp3", ".mp4", ".mpeg", ".mpga", ".m4a", ".wav", ".webm", ".ogg", ".aac", ".flac"];
const MAX_FILE_SIZE: u64 = 25 * 1024 * 1024;

fn get_groq_api_key(config: &Option<AppConfig>) -> Option<String> {
    if let Some(cfg) = config {
        if let Some(ref key) = cfg.tools.stt.groq_api_key {
            if !key.is_empty() {
                return Some(key.clone());
            }
        }
    }
    std::env::var("GROQ_API_KEY").ok()
}

fn get_openai_api_key(config: &Option<AppConfig>) -> Option<String> {
    if let Some(cfg) = config {
        if let Some(ref key) = cfg.tools.stt.openai_api_key {
            if !key.is_empty() {
                return Some(key.clone());
            }
        }
    }
    std::env::var("OPENAI_API_KEY").or_else(|_| std::env::var("VOICE_TOOLS_OPENAI_KEY")).ok()
}

async fn transcribe_groq(file_path: &str, model: &str, api_key: &str) -> TranscriptionResult {
    let audio_bytes = match fs::read(file_path).await {
        Ok(b) => b,
        Err(e) => return TranscriptionResult {
            success: false, transcript: String::new(), provider: "groq".to_string(),
            model: model.to_string(),
            error: Some(format!("Failed to read file: {e}")),
        },
    };

    let file_name = std::path::Path::new(file_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "audio".to_string());

    let file_part = Part::bytes(audio_bytes)
        .file_name(file_name)
        .mime_str("audio/mpeg").unwrap_or_else(|_| Part::bytes(Vec::new()));

    let form = Form::new()
        .part("file", file_part)
        .text("model", model.to_string())
        .text("response_format", "text");

    let client = reqwest::Client::new();
    match client.post("https://api.groq.com/openai/v1/audio/transcriptions")
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
                    success: false, transcript: String::new(), provider: "groq".to_string(),
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
                error: if text.is_empty() { Some("Empty response".to_string()) } else { None },
            }
        }
        Err(e) => TranscriptionResult {
            success: false, transcript: String::new(), provider: "groq".to_string(),
            model: model.to_string(),
            error: Some(format!("Groq request failed: {e}")),
        },
    }
}

async fn transcribe_openai(file_path: &str, model: &str, api_key: &str) -> TranscriptionResult {
    let audio_bytes = match fs::read(file_path).await {
        Ok(b) => b,
        Err(e) => return TranscriptionResult {
            success: false, transcript: String::new(), provider: "openai".to_string(),
            model: model.to_string(),
            error: Some(format!("Failed to read file: {e}")),
        },
    };

    let file_name = std::path::Path::new(file_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "audio".to_string());

    let file_part = Part::bytes(audio_bytes)
        .file_name(file_name)
        .mime_str("audio/mpeg").unwrap_or_else(|_| Part::bytes(Vec::new()));

    let form = Form::new()
        .part("file", file_part)
        .text("model", model.to_string())
        .text("response_format", "text");

    let client = reqwest::Client::new();
    match client.post("https://api.openai.com/v1/audio/transcriptions")
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
                    success: false, transcript: String::new(), provider: "openai".to_string(),
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
                error: if text.is_empty() { Some("Empty response".to_string()) } else { None },
            }
        }
        Err(e) => TranscriptionResult {
            success: false, transcript: String::new(), provider: "openai".to_string(),
            model: model.to_string(),
            error: Some(format!("OpenAI request failed: {e}")),
        },
    }
}

#[async_trait]
impl HermesTool for TranscriptionTool {
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

        let ext = path.extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}").to_lowercase())
            .unwrap_or_default();
        if !SUPPORTED_FORMATS.contains(&ext.as_str()) {
            return ToolResult::error(self.name(),
                format!("Unsupported format '{ext}'. Supported: {}", SUPPORTED_FORMATS.join(", ")));
        }

        match fs::metadata(&parsed.file_path).await {
            Ok(meta) => {
                if meta.len() > MAX_FILE_SIZE {
                    return ToolResult::error(self.name(),
                        format!("File too large: {:.1}MB (max {}MB)", meta.len() as f64 / 1_048_576.0, MAX_FILE_SIZE / 1_048_576));
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
                    None => return ToolResult::error(self.name(), "GROQ_API_KEY not set. Set groqApiKey in tool settings or GROQ_API_KEY env var"),
                };
                let model = if model.is_empty() { "whisper-large-v3-turbo" } else { model };
                transcribe_groq(&parsed.file_path, model, &api_key).await
            }
            "openai" => {
                let api_key = match get_openai_api_key(&self.config) {
                    Some(k) => k,
                    None => return ToolResult::error(self.name(), "OPENAI_API_KEY not set"),
                };
                let model = if model.is_empty() { "whisper-1" } else { model };
                transcribe_openai(&parsed.file_path, model, &api_key).await
            }
            other => return ToolResult::error(self.name(), format!("Unknown provider: '{other}'. Use: groq, openai")),
        };

        ToolResult::success(self.name(), result)
    }
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
        let result = tool
            .execute(json!({}), ToolContext::default())
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_transcription_file_not_found() {
        let tool = TranscriptionTool::new();
        let result = tool
            .execute(json!({"filePath": "/nonexistent/file.mp3"}), ToolContext::default())
            .await;
        assert!(!result.success);
    }
}
