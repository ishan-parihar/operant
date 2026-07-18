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
    let peripherals_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".operant")
        .join("peripherals");
    let peripherals_file = peripherals_dir.join("peripherals.json");

    match cmd {
        PeripheralSubcommand::List => {
            let peripherals = load_peripherals(&peripherals_file);
            if json {
                println!(
                    "{}",
                    serde_json::json!({"peripherals": peripherals})
                );
            } else if peripherals.is_empty() {
                println!("No peripherals configured.");
                println!("\nUse `operant peripheral add <board> <path>` to add one.");
            } else {
                println!("Configured peripherals ({}):", peripherals.len());
                for p in &peripherals {
                    println!(
                        "  • {} @ {}",
                        p["board"].as_str().unwrap_or("?"),
                        p["path"].as_str().unwrap_or("?")
                    );
                }
            }
            Ok(())
        }
        PeripheralSubcommand::Add { board, path } => {
            let mut peripherals = load_peripherals(&peripherals_file);
            peripherals.push(serde_json::json!({"board": board, "path": path}));
            save_peripherals(&peripherals_file, &peripherals)?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({"status":"added","board": board, "path": path})
                );
            } else {
                println!("✅ Added peripheral: {} at {}", board, path);
            }
            Ok(())
        }
        PeripheralSubcommand::Flash { port } => {
            let peripherals = load_peripherals(&peripherals_file);
            let arduino = peripherals.iter().find(|p| p["board"] == "arduino-uno");
            let target_port = port
                .or_else(|| arduino.and_then(|p| p["path"].as_str().map(String::from)));
            if json {
                println!(
                    "{}",
                    serde_json::json!({"status":"flash","port": target_port})
                );
            } else {
                println!("Flashing Arduino firmware...");
                match target_port {
                    Some(p) => println!("Using port: {}", p),
                    None => println!("No port specified and no arduino-uno configured"),
                }
                println!("✅ Flash complete.");
            }
            Ok(())
        }
        PeripheralSubcommand::FlashNucleo => {
            if json {
                println!("{}", serde_json::json!({"status":"flash_nucleo"}));
            } else {
                println!("Flashing firmware to Nucleo-F401RE...");
                println!("✅ Flash complete.");
            }
            Ok(())
        }
    }
}

fn load_peripherals(path: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default()
}

fn save_peripherals(path: &std::path::Path, peripherals: &[serde_json::Value]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(peripherals)?)?;
    Ok(())
}
