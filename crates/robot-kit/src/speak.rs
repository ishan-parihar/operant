use async_trait::async_trait;
use serde_json::Value;

use crate::config::RobotConfig;
use crate::traits::{Tool, ToolResult, ToolSpec};

/// Speak tool for text-to-speech
pub struct SpeakTool {
    config: RobotConfig,
}

impl SpeakTool {
    pub fn new(config: RobotConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for SpeakTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "robot_speak".to_string(),
            description: "Speak text aloud through speaker".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Text to speak" },
                    "voice": { "type": "string", "description": "Voice name (optional)" }
                },
                "required": ["text"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> ToolResult {
        let text = args["text"].as_str().unwrap_or("");
        let voice = args["voice"]
            .as_str()
            .or(self.config.speak.voice.as_deref());
        // TODO: implement actual TTS
        ToolResult::ok(format!(
            "[mock] Speaking via {:?}: \"{}\"",
            voice.unwrap_or("default"),
            text
        ))
    }
}
