use async_trait::async_trait;
use serde_json::Value;

use crate::config::RobotConfig;
use crate::traits::{Tool, ToolResult, ToolSpec};

/// Look tool for camera capture and vision
pub struct LookTool {
    config: RobotConfig,
}

impl LookTool {
    pub fn new(config: RobotConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for LookTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "robot_look".to_string(),
            description: "Capture image from camera and describe what you see".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "What to look for" }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> ToolResult {
        let prompt = args["prompt"].as_str().unwrap_or("describe the scene");
        let device = &self.config.look.device;
        // TODO: implement actual camera capture + vision model
        ToolResult::ok(format!("[mock] Looking at {} with prompt: {}", device, prompt))
    }
}
