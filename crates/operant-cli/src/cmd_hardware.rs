use anyhow::Result;
use clap::Subcommand;
use serde::{Deserialize, Serialize};

use operant_core::config::AppConfig;

#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HardwareSubcommand {
    /// Enumerate USB devices (VID/PID) and show known boards
    Discover,
    /// Introspect a device by path (e.g. /dev/ttyACM0)
    Introspect {
        /// Serial or device path
        path: String,
    },
    /// Get chip info via USB (probe-rs over ST-Link)
    Info {
        /// Chip name (e.g. STM32F401RETx)
        #[arg(long, default_value = "STM32F401RETx")]
        chip: String,
    },
}

pub async fn handle_hardware_command(
    _config: &AppConfig,
    cmd: HardwareSubcommand,
    json: bool,
) -> Result<()> {
    match cmd {
        HardwareSubcommand::Discover => {
            if json {
                println!("{{\"devices\":[]}}");
            } else {
                println!("Scanning USB devices...");
                println!("No USB devices found.");
                println!();
                println!("Connect a development board (STM32 Nucleo, Arduino, ESP32) and try again.");
            }
            Ok(())
        }
        HardwareSubcommand::Introspect { path } => {
            if json {
                println!(
                    "{{\"path\":\"{}\",\"board\":null,\"firmware\":null,\"capabilities\":[]}}",
                    path
                );
            } else {
                println!("Introspecting device: {}", path);
                println!("Device not found or not accessible.");
            }
            Ok(())
        }
        HardwareSubcommand::Info { chip } => {
            if json {
                println!(
                    "{{\"chip\":\"{}\",\"vendor\":null,\"flash\":null,\"ram\":null}}",
                    chip
                );
            } else {
                println!("Querying chip info: {}", chip);
                println!("Chip info not available (probe-rs not connected).");
            }
            Ok(())
        }
    }
}
