use async_trait::async_trait;
use serde_json::Value;

use crate::traits::{Tool, ToolResult, ToolSpec};

/// Sense tool for LIDAR, ultrasonic, motion sensors
#[derive(Default)]
pub struct SenseTool;

impl SenseTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for SenseTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "robot_sense".to_string(),
            description: "Read sensor data (distance, motion, LIDAR)".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "sensor": { "type": "string", "enum": ["distance", "motion", "lidar"] }
                },
                "required": ["sensor"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> ToolResult {
        let sensor = args["sensor"].as_str().unwrap_or("distance");
        match sensor {
            "distance" => ToolResult::ok(serde_json::json!({"distance_cm": 150.0, "sensor": "ultrasonic"}).to_string()),
            "motion" => ToolResult::ok(serde_json::json!({"motion_detected": false, "sensor": "pir"}).to_string()),
            "lidar" => ToolResult::ok(serde_json::json!({"points": [], "sensor": "lidar"}).to_string()),
            _ => ToolResult::err(format!("Unknown sensor: {}", sensor)),
        }
    }
}
