use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::config::AppConfig;

#[derive(Debug, Clone, Subcommand)]
pub enum BackupSubcommand {
    Create {
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    List,
}

pub async fn handle_backup_command(config: &AppConfig, cmd: BackupSubcommand) -> Result<()> {
    match cmd {
        BackupSubcommand::Create { output } => cmd_create(config, output).await,
        BackupSubcommand::List => cmd_list(config).await,
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

async fn cmd_create(_config: &AppConfig, output: Option<PathBuf>) -> Result<()> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let backup_name = format!("operant_backup_{}", secs);
    let base = output.unwrap_or_else(|| std::env::current_dir().unwrap());
    let backup_dir = base.join(&backup_name);
    fs::create_dir_all(&backup_dir)?;

    let config_dir = operant_core::platform::operant_config_dir();
    if config_dir.exists() {
        copy_dir_recursive(&config_dir, &backup_dir.join("config"))
            .context("Failed to copy config directory")?;
        println!("  Backed up config: {}", config_dir.display());
    }

    let data_dir = operant_core::platform::operant_data_dir();
    if data_dir.exists() {
        copy_dir_recursive(&data_dir, &backup_dir.join("data"))
            .context("Failed to copy data directory")?;
        println!("  Backed up data: {}", data_dir.display());
    }

    println!("Backup created: {}", backup_dir.display());
    Ok(())
}

async fn cmd_list(_config: &AppConfig) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let mut found = false;
    for entry in fs::read_dir(&cwd)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with("operant_backup_") && entry.file_type()?.is_dir() {
            let meta = entry.metadata()?;
            let secs = meta
                .modified()
                .unwrap_or(SystemTime::UNIX_EPOCH)
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            println!("  {}  ({})", name_str, secs);
            found = true;
        }
    }
    if !found {
        println!("No backups found in {}", cwd.display());
    }
    Ok(())
}
