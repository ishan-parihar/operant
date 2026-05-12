use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

use crate::schema::ToolSchema;
use crate::tools::{HermesTool, ToolContext, ToolResult};

const DEFAULT_NEUTTS_API_URL: &str = "http://localhost:8020";

pub struct NeuttsSynthTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NeuttsSynthArgs {
    text: String,
    voice: Option<String>,
    speed: Option<f32>,
}

#[derive(Deserialize)]
struct NeuttsResponse {
    #[serde(default)]
    audio_url: Option<String>,
    #[serde(default)]
    audio_base64: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[async_trait]
impl HermesTool for NeuttsSynthTool {
    fn name(&self) -> &str {
        "neutts_synthesize"
    }

    fn description(&self) -> &str {
        "Synthesize speech from text using the NEU TTS (Neural Text-to-Speech) engine. \
         The API URL can be configured via the NEUTTS_API_URL environment variable \
         (default: http://localhost:8020). Returns audio as a URL or base64-encoded data."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<NeuttsSynthArgs>(
            "neutts_synthesize",
            "Synthesize speech using NEU TTS",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: NeuttsSynthArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("neutts_synthesize", format!("Invalid arguments: {}", e))
            }
        };

        let api_url =
            std::env::var("NEUTTS_API_URL").unwrap_or_else(|_| DEFAULT_NEUTTS_API_URL.to_string());

        let endpoint = format!("{}/synthesize", api_url.trim_end_matches('/'));

        let mut body = serde_json::json!({
            "text": args.text
        });

        if let Some(voice) = args.voice {
            body["voice"] = serde_json::json!(voice);
        }
        if let Some(speed) = args.speed {
            body["speed"] = serde_json::json!(speed);
        }

        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::error(
                    "neutts_synthesize",
                    format!("Failed to create HTTP client: {}", e),
                )
            }
        };

        let start = std::time::Instant::now();

        match client
            .post(&endpoint)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
        {
            Ok(response) => {
                let elapsed = start.elapsed();
                let status = response.status().as_u16();

                if !response.status().is_success() {
                    let error_text = response.text().await.unwrap_or_default();
                    return ToolResult::error(
                        "neutts_synthesize",
                        format!("TTS API error ({}): {}", status, error_text),
                    );
                }

                match response.json::<NeuttsResponse>().await {
                    Ok(tts_response) => {
                        if let Some(err) = tts_response.error {
                            return ToolResult::error(
                                "neutts_synthesize",
                                format!("TTS engine error: {}", err),
                            );
                        }

                        ToolResult::success(
                            "neutts_synthesize",
                            serde_json::json!({
                                "audio_url": tts_response.audio_url,
                                "audio_base64": tts_response.audio_base64,
                                "response_time_ms": elapsed.as_millis() as u64,
                                "text_length": args.text.len()
                            }),
                        )
                    }
                    Err(e) => ToolResult::error(
                        "neutts_synthesize",
                        format!("Failed to parse TTS response: {}", e),
                    ),
                }
            }
            Err(e) => ToolResult::error("neutts_synthesize", format!("TTS request failed: {}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_neutts_invalid_args() {
        let tool = NeuttsSynthTool;
        let result = tool
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_neutts_deserialize() {
        let tool = NeuttsSynthTool;
        let args = serde_json::json!({
            "text": "Hello world",
            "voice": "en_us_female",
            "speed": 1.0
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(!result.success);
    }
}
