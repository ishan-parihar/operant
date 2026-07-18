use serde::{Deserialize, Serialize};

/// Configuration for the robot kit
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RobotConfig {
    /// Drive configuration
    #[serde(default)]
    pub drive: DriveConfig,
    /// Look configuration
    #[serde(default)]
    pub look: LookConfig,
    /// Listen configuration
    #[serde(default)]
    pub listen: ListenConfig,
    /// Speak configuration
    #[serde(default)]
    pub speak: SpeakConfig,
    /// Sense configuration
    #[serde(default)]
    pub sense: SenseConfig,
}

/// Drive motor control configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveConfig {
    /// Backend: mock, serial, gpio, ros2
    pub backend: String,
    /// Serial port path (if backend = serial)
    pub serial_port: Option<String>,
    /// Max speed (0.0 - 1.0)
    pub max_speed: f64,
}

impl Default for DriveConfig {
    fn default() -> Self {
        Self {
            backend: "mock".to_string(),
            serial_port: None,
            max_speed: 1.0,
        }
    }
}

/// Camera / vision configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookConfig {
    /// Camera device index or path
    pub device: String,
    /// Vision model for image description
    pub model: Option<String>,
}

impl Default for LookConfig {
    fn default() -> Self {
        Self {
            device: "/dev/video0".to_string(),
            model: None,
        }
    }
}

/// Microphone / STT configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListenConfig {
    /// Microphone device
    pub device: String,
    /// STT model
    pub model: String,
}

impl Default for ListenConfig {
    fn default() -> Self {
        Self {
            device: "default".to_string(),
            model: "whisper-base".to_string(),
        }
    }
}

/// Speaker / TTS configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakConfig {
    /// TTS provider
    pub provider: String,
    /// Voice name
    pub voice: Option<String>,
}

impl Default for SpeakConfig {
    fn default() -> Self {
        Self {
            provider: "mock".to_string(),
            voice: None,
        }
    }
}

/// Sensor configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SenseConfig {
    /// LIDAR device
    pub lidar_device: Option<String>,
    /// Ultrasonic GPIO pins
    pub ultrasonic_pins: Option<(u8, u8)>,
}
