use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

pub struct TruncateOutputTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TruncateOutputArgs {
    content: String,
    max_chars: Option<u32>,
    max_lines: Option<u32>,
}

#[async_trait]
impl OperantTool for TruncateOutputTool {
    fn name(&self) -> &str {
        "apply_output_limits"
    }

    fn description(&self) -> &str {
        "Truncate tool output to fit within specified character and/or line limits. \
         Appends a '[truncated ...]' message when content is trimmed."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<TruncateOutputArgs>(
            "apply_output_limits",
            "Truncate tool output to limits",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: TruncateOutputArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error(
                    "apply_output_limits",
                    format!("Invalid arguments: {}", e),
                )
            }
        };

        let original_len = args.content.len();
        let original_lines = args.content.lines().count();

        let mut content = args.content;
        let mut chars_truncated = 0u64;
        let mut lines_truncated = 0u64;

        if let Some(max_lines) = args.max_lines {
            let max_lines = max_lines as usize;
            let line_count = content.lines().count();
            if line_count > max_lines {
                let truncated: String = content
                    .lines()
                    .take(max_lines)
                    .collect::<Vec<&str>>()
                    .join("\n");
                lines_truncated = (line_count - max_lines) as u64;
                content = truncated;
            }
        }

        if let Some(max_chars) = args.max_chars {
            let max_chars = max_chars as usize;
            if content.chars().count() > max_chars {
                let truncated: String = content.chars().take(max_chars).collect();
                chars_truncated = (content.chars().count() - max_chars) as u64;
                content = truncated;
            }
        }

        if chars_truncated > 0 || lines_truncated > 0 {
            let parts: Vec<String> = [
                if chars_truncated > 0 {
                    Some(format!("{} chars", chars_truncated))
                } else {
                    None
                },
                if lines_truncated > 0 {
                    Some(format!("{} lines", lines_truncated))
                } else {
                    None
                },
            ]
            .into_iter()
            .flatten()
            .collect();

            content.push_str(&format!("\n... [truncated {}]", parts.join(", ")));
        }

        ToolResult::success(
            "apply_output_limits",
            serde_json::json!({
                "content": content,
                "original_length": original_len,
                "original_lines": original_lines,
                "chars_truncated": chars_truncated,
                "lines_truncated": lines_truncated,
                "was_truncated": chars_truncated > 0 || lines_truncated > 0
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_truncate_no_limits() {
        let tool = TruncateOutputTool;
        let args = serde_json::json!({
            "content": "hello world"
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["content"], "hello world");
        assert_eq!(v["was_truncated"], false);
    }

    #[tokio::test]
    async fn test_truncate_max_chars() {
        let tool = TruncateOutputTool;
        let args = serde_json::json!({
            "content": "hello world this is a long string",
            "maxChars": 10
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert!(v["content"].as_str().unwrap().contains("truncated"));
        assert_eq!(v["was_truncated"], true);
    }

    #[tokio::test]
    async fn test_truncate_max_lines() {
        let tool = TruncateOutputTool;
        let args = serde_json::json!({
            "content": "line1\nline2\nline3\nline4\nline5",
            "maxLines": 3
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert!(v["content"].as_str().unwrap().contains("truncated"));
        assert_eq!(v["was_truncated"], true);
    }

    #[tokio::test]
    async fn test_truncate_both_limits() {
        let tool = TruncateOutputTool;
        let args = serde_json::json!({
            "content": "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np",
            "maxChars": 20,
            "maxLines": 5
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert!(v["content"].as_str().unwrap().contains("truncated"));
        assert_eq!(v["was_truncated"], true);
    }
}
