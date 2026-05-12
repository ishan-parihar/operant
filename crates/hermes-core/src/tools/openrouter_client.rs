use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use crate::schema::ToolSchema;
use crate::tools::{HermesTool, ToolContext, ToolResult};

const OPENROUTER_API_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

pub struct OpenRouterTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenRouterArgs {
    model: String,
    prompt: String,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
}

#[derive(Serialize)]
struct OpenRouterRequest {
    model: String,
    messages: Vec<OpenRouterMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Serialize)]
struct OpenRouterMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OpenRouterResponse {
    #[serde(default)]
    choices: Vec<OpenRouterChoice>,
    #[serde(default)]
    usage: Option<OpenRouterUsage>,
    #[serde(default)]
    error: Option<OpenRouterError>,
}

#[derive(Deserialize)]
struct OpenRouterChoice {
    message: OpenRouterResponseMessage,
}

#[derive(Deserialize)]
struct OpenRouterResponseMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct OpenRouterUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
}

#[derive(Deserialize)]
struct OpenRouterError {
    message: Option<String>,
}

#[async_trait]
impl HermesTool for OpenRouterTool {
    fn name(&self) -> &str {
        "openrouter_query"
    }

    fn description(&self) -> &str {
        "Query the OpenRouter API for model inference with automatic fallback. \
         Requires the OPENROUTER_API_KEY environment variable to be set. \
         Supports specifying model, prompt, max_tokens, and temperature."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<OpenRouterArgs>(
            "openrouter_query",
            "Query OpenRouter API for model inference",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: OpenRouterArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error(
                    "openrouter_query",
                    format!("Invalid arguments: {}", e),
                )
            }
        };

        let api_key = match std::env::var("OPENROUTER_API_KEY") {
            Ok(key) => key,
            Err(_) => {
                return ToolResult::error(
                    "openrouter_query",
                    "OPENROUTER_API_KEY environment variable not set",
                )
            }
        };

        let request_body = OpenRouterRequest {
            model: args.model,
            messages: vec![OpenRouterMessage {
                role: "user".to_string(),
                content: args.prompt,
            }],
            max_tokens: args.max_tokens,
            temperature: args.temperature,
        };

        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::error(
                    "openrouter_query",
                    format!("Failed to create HTTP client: {}", e),
                )
            }
        };

        let body_json = match serde_json::to_value(&request_body) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::error(
                    "openrouter_query",
                    format!("Failed to serialize request: {}", e),
                )
            }
        };

        let start = std::time::Instant::now();

        let response = match client
            .post(OPENROUTER_API_URL)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error(
                    "openrouter_query",
                    format!("Request failed: {}", e),
                )
            }
        };

        let elapsed = start.elapsed();
        let status = response.status().as_u16();

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return ToolResult::error(
                "openrouter_query",
                format!("API error ({}): {}", status, error_text),
            );
        }

        let response_body: OpenRouterResponse = match response.json().await {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error(
                    "openrouter_query",
                    format!("Failed to parse response: {}", e),
                )
            }
        };

        if let Some(err) = response_body.error {
            return ToolResult::error(
                "openrouter_query",
                err.message.unwrap_or_else(|| "Unknown API error".to_string()),
            );
        }

        let response_text = response_body
            .choices
            .first()
            .and_then(|c| c.message.content.as_ref())
            .map(|s| s.clone())
            .unwrap_or_default();

        let usage = response_body.usage.map(|u| {
            serde_json::json!({
                "prompt_tokens": u.prompt_tokens,
                "completion_tokens": u.completion_tokens,
                "total_tokens": u.total_tokens
            })
        });

        ToolResult::success(
            "openrouter_query",
            serde_json::json!({
                "model": body_json["model"],
                "response": response_text,
                "usage": usage,
                "response_time_ms": elapsed.as_millis() as u64
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_openrouter_invalid_args() {
        let tool = OpenRouterTool;
        let result = tool
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_openrouter_missing_api_key() {
        let tool = OpenRouterTool;
        std::env::remove_var("OPENROUTER_API_KEY");
        let args = serde_json::json!({
            "model": "gpt-4",
            "prompt": "Hello"
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(!result.success);
        assert!(result.error.unwrap().contains("OPENROUTER_API_KEY"));
    }
}
