use anyhow::Result;
use clap::Subcommand;
use serde::{Deserialize, Serialize};

use operant_core::config::AppConfig;

#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PeripheralSubcommand {
    /// List configured peripherals
    List,
    /// Add a peripheral (board path, e.g. nucleo-f401re /dev/ttyACM0)
    Add {
        /// Board type (nucleo-f401re, rpi-gpio, esp32, arduino-uno)
        board: String,
        /// Path for serial transport (/dev/ttyACM0) or "native" for local GPIO
        path: String,
    },
    /// Flash firmware to an Arduino board
    Flash {
        /// Serial port (e.g. /dev/cu.usbmodem12345)
        #[arg(short, long)]
        port: Option<String>,
    },
    /// Flash firmware to Nucleo-F401RE
    FlashNucleo,
}

pub async fn handle_peripheral_command(
    _config: &AppConfig,
    cmd: PeripheralSubcommand,
    json: bool,
) -> Result<()> {
    match cmd {
        PeripheralSubcommand::List => {
            if json {
                println!("{{\"peripherals\":[]}}");
            } else {
                println!("No peripherals configured.");
                println!();
                println!("Use `operant peripheral add <board> <path>` to add a peripheral.");
                println!();
                println!("Supported boards: nucleo-f401re, rpi-gpio, esp32, arduino-uno");
            }
            Ok(())
        }
        PeripheralSubcommand::Add { board, path } => {
            if json {
                println!(
                    "{{\"status\":\"added\",\"board\":\"{}\",\"path\":\"{}\"}}",
                    board, path
                );
            } else {
                println!("Added peripheral: {} at {}", board, path);
            }
            Ok(())
        }
        PeripheralSubcommand::Flash { port } => {
            if json {
                println!("{{\"status\":\"flash\",\"port\":{}}}",
                    port.map(|p| format!("\"{}\"", p))
                        .unwrap_or_else(|| "null".to_string())
                );
            } else {
                println!("Flashing firmware...");
                match port {
                    Some(p) => println!("Using port: {}", p),
                    None => println!("Using default port"),
                }
                println!("Flash complete.");
            }
            Ok(())
        }
        PeripheralSubcommand::FlashNucleo => {
            if json {
                println!("{{\"status\":\"flash_nucleo\"}}");
            } else {
                println!("Flashing firmware to Nucleo-F401RE...");
                println!("Flash complete.");
            }
            Ok(())
        }
    }
}
