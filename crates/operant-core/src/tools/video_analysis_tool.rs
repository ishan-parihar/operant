use async_trait::async_trait;
use reqwest::Client;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

pub struct VideoAnalysisTool {
    client: Client,
    fal_key: String,
}

/// The analysis-mode knob that used to sit here was REMOVED (RT-344): it
/// was deserialized and never read - the request body carries only
/// `video_url` and `prompt`, so the schema advertised a setting that
/// silently did nothing. Re-add it only with a real payload mapping. (The
/// knob's name is deliberately not spelled here: this doc IS the schema's
/// description, so spelling it would put the dead setting back in the
/// schema the agent reads.)
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoAnalysisArgs {
    pub video_url: String,
    pub prompt: Option<String>,
}

impl Default for VideoAnalysisTool {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoAnalysisTool {
    pub fn new() -> Self {
        let fal_key = std::env::var("FAL_KEY").unwrap_or_default();
        Self {
            client: Client::new(),
            fal_key,
        }
    }

    async fn analyze_video(&self, args: &VideoAnalysisArgs) -> ToolResult {
        if self.fal_key.is_empty() {
            return ToolResult::error("video_analyze", "FAL_KEY not set");
        }

        if args.video_url.trim().is_empty() {
            return ToolResult::error("video_analyze", "Video URL is required");
        }

        let response = match self
            .client
            .post("https://queue.fal.run/fal-ai/video-analysis")
            .header("Authorization", format!("Key {}", self.fal_key))
            .header("Content-Type", "application/json")
            .json(&json!({
                "video_url": args.video_url,
                "prompt": args.prompt.clone().unwrap_or_else(|| "Describe what's happening in this video".to_string())
            }))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return ToolResult::error("video_analyze", format!("API request failed: {}", e)),
        };

        if !response.status().is_success() {
            return ToolResult::error("video_analyze", format!("API error: {}", response.status()));
        }

        let result: Value = match response.json().await {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error(
                    "video_analyze",
                    format!("Failed to parse response: {}", e),
                );
            }
        };

        let analysis = result
            .get("analysis")
            .and_then(|a| a.as_str())
            .unwrap_or("No analysis returned");

        ToolResult::success(
            "video_analyze",
            json!({
                "success": true,
                "analysis": analysis,
                "video_url": args.video_url
            }),
        )
    }
}

#[async_trait]
impl OperantTool for VideoAnalysisTool {
    fn name(&self) -> &str {
        "video_analyze"
    }

    fn description(&self) -> &str {
        "Analyze videos using AI to describe content, detect objects, or extract information"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<VideoAnalysisArgs>("video_analyze", "Analyze videos using AI")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: VideoAnalysisArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("video_analyze", format!("Invalid arguments: {}", e));
            }
        };
        self.analyze_video(&args).await
    }
}

#[cfg(test)]
mod tests {
    /// RT-344: `analysis_type` was deserialized and never read -- the request
    /// body carries only `video_url` and `prompt`, so the schema advertised a
    /// setting that silently did nothing. The schema must not offer it again,
    /// and no code may read it back in.
    #[test]
    fn the_schema_does_not_advertise_a_knob_no_request_reads() {
        let schema = VideoAnalysisTool::new().schema();
        let rendered = serde_json::to_string(&schema.parameters).expect("schema json");
        // The needle is built by concatenation so this test's own source
        // cannot satisfy it.
        let knob = format!("analysis{}", "_type");
        assert!(
            !rendered.contains(&knob),
            "the dead knob must not be in the schema: {rendered}"
        );
        let src = include_str!("video_analysis_tool.rs");
        let any_read = format!("args.{}", "analysis_type");
        assert_eq!(
            src.matches(&any_read).count(),
            0,
            "no code may read the retracted field"
        );
    }

    use super::*;
    use serde_json::json;

    #[test]
    fn test_video_analysis_schema() {
        let tool = VideoAnalysisTool::new();
        let schema = tool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "video_analyze");
    }

    #[tokio::test]
    async fn test_video_analysis_invalid_args() {
        let tool = VideoAnalysisTool::new();
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success);
    }
}
