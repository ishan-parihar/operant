use async_trait::async_trait;
use serde_json::Value;

use crate::traits::{Tool, ToolResult, ToolSpec};

/// Listen tool for speech-to-text
#[derive(Default)]
pub struct ListenTool;

impl ListenTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ListenTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "robot_listen".to_string(),
            description: "Listen to audio from microphone and transcribe".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "duration_secs": { "type": "number", "description": "How long to listen (seconds)" }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> ToolResult {
        let duration = args["duration_secs"].as_f64().unwrap_or(5.0);
        // TODO: implement actual STT
        ToolResult::ok(format!("[mock] Listened for {}s, no speech detected", duration))
    }
}
