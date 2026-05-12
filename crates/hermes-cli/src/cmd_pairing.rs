use std::fs;

use anyhow::Result;
use clap::Subcommand;
use hermes_core::config::AppConfig;

#[derive(Debug, Clone, Subcommand)]
pub enum PairingSubcommand {
    /// List all paired devices
    List,
    /// Approve a pending pairing request
    Approve {
        /// Pairing code
        code: String,
    },
    /// Revoke a paired device
    Revoke {
        /// Device ID to revoke
        device_id: String,
    },
    /// Clear all pending pairing requests
    ClearPending,
}

pub async fn handle_pairing_command(_config: &AppConfig, cmd: PairingSubcommand) -> Result<()> {
    let dir = hermes_core::platform::hermes_data_dir().join("pairing");
    match cmd {
        PairingSubcommand::List => {
            if !dir.exists() {
                println!("No pairings found.");
                return Ok(());
            }
            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("approved_") {
                    println!("  {}  (approved)", name.trim_start_matches("approved_"));
                } else {
                    println!("  {}  (pending)", name);
                }
            }
        }
        PairingSubcommand::Approve { code } => {
            fs::create_dir_all(&dir)?;
            fs::write(dir.join(format!("approved_{}", code)), "approved")?;
            println!("Pairing '{}' approved.", code);
        }
        PairingSubcommand::Revoke { device_id } => {
            let file = dir.join(format!("approved_{}", device_id));
            if file.exists() {
                fs::remove_file(&file)?;
                println!("Pairing '{}' revoked.", device_id);
            } else {
                println!("No pairing found for '{}'.", device_id);
            }
        }
        PairingSubcommand::ClearPending => {
            if dir.exists() {
                for entry in fs::read_dir(&dir)? {
                    let entry = entry?;
                    let name = entry.file_name().to_string_lossy().to_string();
                    if !name.starts_with("approved_") {
                        fs::remove_file(entry.path())?;
                    }
                }
            }
            println!("Pending pairing requests cleared.");
        }
    }
    Ok(())
}
