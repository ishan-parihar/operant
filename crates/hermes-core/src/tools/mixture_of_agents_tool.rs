use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::schema::ToolSchema;
use crate::tools::{HermesTool, ToolContext, ToolResult};

pub struct MixtureOfAgentsTool;

const REFERENCE_MODELS: &[&str] = &[
    "anthropic/claude-opus-4.6",
    "google/gemini-2.5-pro-preview-03-25",
    "openai/gpt-5.4-pro",
    "deepseek/deepseek-v3.2",
];
const AGGREGATOR_MODEL: &str = "anthropic/claude-opus-4.6";
const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct MoAArgs {
    prompt: String,
    #[serde(default)]
    max_tokens: Option<u32>,
}

#[async_trait]
impl HermesTool for MixtureOfAgentsTool {
    fn name(&self) -> &str {
        "mixture_of_agents"
    }

    fn description(&self) -> &str {
        "Run a prompt through multiple frontier models (Claude, Gemini, GPT, DeepSeek) and aggregate their responses for improved quality using a two-layer architecture."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<MoAArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let api_key = match std::env::var("OPENROUTER_API_KEY") {
            Ok(key) => key,
            Err(_) => return ToolResult::error(self.name(), "OPENROUTER_API_KEY not set"),
        };

        let parsed: MoAArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid args: {}", e)),
        };

        let max_tokens = parsed.max_tokens.unwrap_or(1024);
        let client = reqwest::Client::new();

        let system_msg = json!({
            "role": "system",
            "content": "You are a helpful assistant."
        });
        let user_msg = json!({
            "role": "user",
            "content": parsed.prompt
        });

        let ref_futures: Vec<_> = REFERENCE_MODELS
            .iter()
            .map(|model| {
                let body = json!({
                    "model": model,
                    "messages": [system_msg, user_msg],
                    "max_tokens": max_tokens,
                });
                call_model_owned(&client, &api_key, body)
            })
            .collect();

        let ref_results: Vec<std::result::Result<String, String>> =
            futures::future::join_all(ref_futures).await;

        let mut ref_responses = Vec::new();
        let mut errors = Vec::new();
        for (i, result) in ref_results.iter().enumerate() {
            match result {
                Ok(content) => {
                    ref_responses.push(format!("[{} Response]:\n{}", REFERENCE_MODELS[i], content))
                }
                Err(e) => {
                    errors.push(format!("{}: {}", REFERENCE_MODELS[i], e));
                    ref_responses.push(format!("[{} Error]: {}", REFERENCE_MODELS[i], e));
                }
            }
        }

        if ref_responses.is_empty() {
            return ToolResult::error(
                self.name(),
                format!("All reference models failed: {:?}", errors),
            );
        }

        let agg_system_msg = json!({
            "role": "system",
            "content": "You are a response aggregator. Synthesize the best answer from the provided responses."
        });
        let agg_user_msg = json!({
            "role": "user",
            "content": format!(
                "Original prompt: {}\n\nResponses from different models:\n\n{}",
                parsed.prompt,
                ref_responses.join("\n\n---\n\n")
            )
        });

        let agg_body = json!({
            "model": AGGREGATOR_MODEL,
            "messages": [agg_system_msg, agg_user_msg],
            "max_tokens": max_tokens * 2,
        });

        match call_model_owned(&client, &api_key, agg_body).await {
            Ok(content) => ToolResult::success(
                self.name(),
                json!({
                    "result": content,
                    "models_used": REFERENCE_MODELS,
                    "aggregator": AGGREGATOR_MODEL,
                    "model_errors": errors,
                }),
            ),
            Err(e) => ToolResult::error(self.name(), format!("Aggregation failed: {}", e)),
        }
    }
}

async fn call_model_owned(
    client: &reqwest::Client,
    api_key: &str,
    body: Value,
) -> std::result::Result<String, String> {
    let resp = client
        .post(OPENROUTER_URL)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP error: {}", e))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("Read error: {}", e))?;

    if !status.is_success() {
        return Err(format!("API error ({}): {}", status, text));
    }

    let json: Value =
        serde_json::from_str(&text).map_err(|e| format!("JSON parse error: {}", e))?;

    json.pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            format!(
                "No content in response: {}",
                text.chars().take(200).collect::<String>()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolContext;
    use serde_json::json;

    #[tokio::test]
    async fn test_mixture_of_agents_schema() {
        let tool = MixtureOfAgentsTool;
        assert_eq!(tool.name(), "mixture_of_agents");
        assert!(!tool.description().is_empty());

        let schema = tool.schema();
        assert_eq!(schema.name, "mixture_of_agents");
        assert!(serde_json::to_string(&schema.parameters).is_ok());
    }

    #[tokio::test]
    async fn test_mixture_of_agents_missing_env() {
        let saved = std::env::var("OPENROUTER_API_KEY").ok();
        std::env::remove_var("OPENROUTER_API_KEY");

        let tool = MixtureOfAgentsTool;
        let result = tool
            .execute(json!({"prompt": "test prompt"}), ToolContext::default())
            .await;

        if let Some(key) = saved {
            std::env::set_var("OPENROUTER_API_KEY", key);
        }
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_mixture_of_agents_missing_prompt() {
        let tool = MixtureOfAgentsTool;
        let result = tool
            .execute(json!("not_an_object"), ToolContext::default())
            .await;

        assert!(!result.success);
    }
}
