//! # Operant Robot Kit
//!
//! A standalone robotics toolkit that integrates with Operant for AI-powered robots.
//!
//! ## Features
//!
//! - **Drive**: Omni-directional motor control (serial, GPIO, mock)
//! - **Look**: Camera capture + vision model description
//! - **Listen**: Speech-to-text via Whisper
//! - **Speak**: Text-to-speech
//! - **Sense**: LIDAR, motion sensors, ultrasonic distance
//! - **Emote**: LED matrix expressions and sound effects

#![allow(missing_docs)]

pub mod config;
pub mod drive;
pub mod emote;
pub mod listen;
pub mod look;
pub mod sense;
pub mod speak;
pub mod traits;

pub use config::RobotConfig;
pub use drive::DriveTool;
pub use emote::EmoteTool;
pub use listen::ListenTool;
pub use look::LookTool;
pub use sense::SenseTool;
pub use speak::SpeakTool;
pub use traits::{Tool, ToolResult, ToolSpec};

/// Crate version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Create all robot tools with default configuration
pub fn create_tools(config: &RobotConfig) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(DriveTool::new(config.clone())),
        Box::new(LookTool::new(config.clone())),
        Box::new(ListenTool::new()),
        Box::new(SpeakTool::new(config.clone())),
        Box::new(SenseTool::new()),
        Box::new(EmoteTool::new()),
    ]
}
