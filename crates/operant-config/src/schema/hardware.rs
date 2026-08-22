//! `hardware` configuration surface — extracted verbatim from the
//! former schema.rs monolith (dedup pass). Placement is navigational;
//! every item is re-exported from `schema::`.

use anyhow::Result;
use operant_macros::Configurable;
#[cfg(feature = "schema-export")]
use serde::{Deserialize, Serialize};

use super::*;

// ── Hardware Config (wizard-driven) ─────────────────────────────

/// Hardware transport mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub enum HardwareTransport {
    #[default]
    /// No hardware transport.
    None,
    /// Native host device access.
    Native,
    /// Serial-port transport.
    Serial,
    /// Probe-based discovery transport.
    Probe,
}

impl std::fmt::Display for HardwareTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Native => write!(f, "native"),
            Self::Serial => write!(f, "serial"),
            Self::Probe => write!(f, "probe"),
        }
    }
}

/// Wizard-driven hardware configuration for physical world interaction.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "hardware"]
pub struct HardwareConfig {
    /// Opt in to direct physical-hardware control — GPIO pins, USB-tethered microcontrollers (Arduino, ESP32, Nucleo), or SWD/JTAG debug probes. Leave off for software-only use; turning it on without the right transport configured does nothing.
    #[serde(default)]
    pub enabled: bool,
    /// How Operant reaches the hardware: `native` = Linux SBC with direct GPIO access (Raspberry Pi, Orange Pi); `serial` = USB-tethered microcontroller speaking over a TTY; `probe` = SWD/JTAG debug probe driving a target chip via probe-rs; `none` = disabled.
    #[serde(default)]
    pub transport: HardwareTransport,
    /// TTY path for the `serial` transport — e.g. `/dev/ttyACM0` on Linux, `/dev/tty.usbmodem1` on macOS, `COM3` on Windows. Ignored for other transports.
    #[serde(default)]
    pub serial_port: Option<String>,
    /// Baud rate negotiated on the serial link. 115200 matches the common Arduino / ESP32 bootloader default; bump to 230400+ when your firmware explicitly supports faster rates and you need the throughput.
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
    /// Target chip identifier for `transport = probe` (e.g. `STM32F401RE`, `nRF52840_xxAA`). Passed straight to probe-rs for flash/debug operations; must match a chip probe-rs recognizes.
    #[serde(default)]
    pub probe_target: Option<String>,
    /// Index PDF schematics and datasheets from the workspace into a local RAG store, so the agent can look up pin assignments and electrical specs inline when you ask hardware questions. Off by default — turn on once the workspace has relevant PDFs dropped in.
    #[serde(default)]
    pub workspace_datasheets: bool,
}

impl HardwareConfig {
    /// Return the active transport mode.
    pub fn transport_mode(&self) -> HardwareTransport {
        self.transport.clone()
    }
}

impl Default for HardwareConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            transport: HardwareTransport::None,
            serial_port: None,
            baud_rate: default_baud_rate(),
            probe_target: None,
            workspace_datasheets: false,
        }
    }
}

// ── Peripherals (hardware: STM32, RPi GPIO, etc.) ────────────────────────

/// Peripheral board integration configuration (`[peripherals]` section).
///
/// Boards become agent tools when enabled.
#[derive(Debug, Clone, Serialize, Deserialize, Default, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "peripherals"]
pub struct PeripheralsConfig {
    /// Enable peripheral support (boards become agent tools)
    #[serde(default)]
    pub enabled: bool,
    /// Board configurations (nucleo-f401re, rpi-gpio, etc.)
    #[serde(default)]
    pub boards: Vec<PeripheralBoardConfig>,
    /// Path to datasheet docs (relative to workspace) for RAG retrieval.
    /// Place .md/.txt files named by board (e.g. nucleo-f401re.md, rpi-gpio.md).
    #[serde(default)]
    pub datasheet_dir: Option<String>,
}

/// Configuration for a single peripheral board (e.g. STM32, RPi GPIO).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct PeripheralBoardConfig {
    /// Board type: "nucleo-f401re", "rpi-gpio", "esp32", etc.
    pub board: String,
    /// Transport: "serial", "native", "websocket"
    #[serde(default = "default_peripheral_transport")]
    pub transport: String,
    /// Path for serial: "/dev/ttyACM0", "/dev/ttyUSB0"
    #[serde(default)]
    pub path: Option<String>,
    /// Baud rate for serial (default: 115200)
    #[serde(default = "default_peripheral_baud")]
    pub baud: u32,
}

impl Default for PeripheralBoardConfig {
    fn default() -> Self {
        Self {
            board: String::new(),
            transport: default_peripheral_transport(),
            path: None,
            baud: default_peripheral_baud(),
        }
    }
}
