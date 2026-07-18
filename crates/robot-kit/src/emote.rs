use async_trait::async_trait;
use serde_json::Value;

use crate::traits::{Tool, ToolResult, ToolSpec};

/// Emote tool for LED expressions and sound effects
#[derive(Default)]
pub struct EmoteTool;

impl EmoteTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for EmoteTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "robot_emote".to_string(),
            description: "Display expression on LED matrix or play sound effect".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "expression": { "type": "string", "enum": ["happy", "sad", "neutral", "alert", "thinking", "wave"] },
                    "sound": { "type": "string", "description": "Sound effect name (optional)" }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> ToolResult {
        let expression = args["expression"].as_str().unwrap_or("neutral");
        let sound = args["sound"].as_str();
        // TODO: implement actual LED/sound control
        ToolResult::ok(format!(
            "[mock] Emoting: {:?}, sound: {:?}",
            expression, sound
        ))
    }
}
