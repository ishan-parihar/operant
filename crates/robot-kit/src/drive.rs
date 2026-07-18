use async_trait::async_trait;
use serde_json::Value;

use crate::config::{DriveConfig, RobotConfig};
use crate::traits::{Tool, ToolResult, ToolSpec};

/// Drive tool for motor control
pub struct DriveTool {
    config: DriveConfig,
}

impl DriveTool {
    pub fn new(config: RobotConfig) -> Self {
        Self {
            config: config.drive,
        }
    }
}

#[async_trait]
impl Tool for DriveTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "robot_drive".to_string(),
            description: "Control robot movement: forward, backward, left, right, stop".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["forward", "backward", "left", "right", "stop"] },
                    "distance": { "type": "number", "description": "Distance in meters (0 = continuous)" },
                    "speed": { "type": "number", "description": "Speed 0.0-1.0" }
                },
                "required": ["action"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> ToolResult {
        let action = args["action"].as_str().unwrap_or("stop");
        let distance = args["distance"].as_f64().unwrap_or(0.0);
        let speed = args["speed"].as_f64().unwrap_or(0.5);

        match self.config.backend.as_str() {
            "mock" => ToolResult::ok(format!(
                "[mock] Drive {} distance={} speed={}",
                action, distance, speed
            )),
            "serial" => {
                // TODO: implement serial motor control
                ToolResult::ok(format!(
                    "[serial] Drive {} distance={} speed={}",
                    action, distance, speed
                ))
            }
            _ => ToolResult::err(format!("Unknown drive backend: {}", self.config.backend)),
        }
    }
}
