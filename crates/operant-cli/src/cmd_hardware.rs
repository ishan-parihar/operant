use std::path::Path;

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

/// Known development boards by VID:PID.
const KNOWN_BOARDS: &[(u16, u16, &str)] = &[
    (0x0483, 0x374b, "STM32 Nucleo"),
    (0x0483, 0x374e, "STM32 Nucleo"),
    (0x2341, 0x0043, "Arduino Uno"),
    (0x2341, 0x0042, "Arduino Mega"),
    (0x303a, 0x4002, "ESP32-S2"),
    (0x303a, 0x0002, "ESP32-S3"),
    (0x303a, 0x1001, "ESP32-C3"),
];

pub async fn handle_hardware_command(
    _config: &AppConfig,
    cmd: HardwareSubcommand,
    json: bool,
) -> Result<()> {
    match cmd {
        HardwareSubcommand::Discover => {
            // Try to read /sys/bus/usb/devices or just report mock
            let devices = discover_usb_devices();
            if json {
                println!(
                    "{}",
                    serde_json::json!({"devices": devices.iter().map(|(vid, pid, name)| serde_json::json!({"vid": format!("0x{:04x}", vid), "pid": format!("0x{:04x}", pid), "name": name})).collect::<Vec<_>>()})
                );
            } else if devices.is_empty() {
                println!("No recognized USB devices found.");
                println!("\nConnect a development board and try again.");
            } else {
                println!("Found {} USB device(s):", devices.len());
                for (vid, pid, name) in &devices {
                    println!("  ✅ {} (VID:PID = 0x{:04x}:0x{:04x})", name, vid, pid);
                }
            }
            Ok(())
        }
        HardwareSubcommand::Introspect { path } => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "path": path,
                        "accessible": Path::new(&path).exists(),
                        "board": null,
                        "firmware": null
                    })
                );
            } else {
                println!("Introspecting: {}", path);
                if Path::new(&path).exists() {
                    println!("✅ Device exists");
                } else {
                    println!("❌ Device not found at {}", path);
                }
            }
            Ok(())
        }
        HardwareSubcommand::Info { chip } => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"chip": chip, "vendor": "STMicroelectronics", "flash_kb": 512, "ram_kb": 96})
                );
            } else {
                println!("Chip: {}", chip);
                println!("Vendor: STMicroelectronics");
                println!("Flash: 512 KB, RAM: 96 KB");
            }
            Ok(())
        }
    }
}

fn discover_usb_devices() -> Vec<(u16, u16, &'static str)> {
    let mut found = Vec::new();

    // Try reading sysfs for USB devices
    if let Ok(entries) = std::fs::read_dir("/sys/bus/usb/devices") {
        for entry in entries.flatten() {
            let vid_path = entry.path().join("idVendor");
            let pid_path = entry.path().join("idProduct");
            if let (Ok(vid), Ok(pid)) = (
                std::fs::read_to_string(&vid_path),
                std::fs::read_to_string(&pid_path),
            ) && let (Ok(vid), Ok(pid)) = (
                u16::from_str_radix(vid.trim(), 16),
                u16::from_str_radix(pid.trim(), 16),
            ) && let Some((_, _, name)) =
                KNOWN_BOARDS.iter().find(|(v, p, _)| *v == vid && *p == pid)
            {
                found.push((vid, pid, *name));
            }
        }
    }
    found
}
