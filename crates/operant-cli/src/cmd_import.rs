use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::config::AppConfig;

#[derive(Debug, Clone, Subcommand)]
pub enum ImportSubcommand {
    /// Import from a backup directory
    Path {
        /// Path to the backup directory
        path: PathBuf,
        /// Overwrite without asking
        #[arg(short, long)]
        force: bool,
    },
}

pub async fn handle_import_command(config: &AppConfig, cmd: ImportSubcommand) -> Result<()> {
    match cmd {
        ImportSubcommand::Path { path, force } => cmd_import(config, &path, force).await,
    }
}

fn copy_dir_recursive(src: &PathBuf, dst: &PathBuf) -> Result<()> {
    fs::create_dir_all(dst)
        .with_context(|| format!("Failed to create destination: {}", dst.display()))?;
    for entry in
        fs::read_dir(src).with_context(|| format!("Failed to read source: {}", src.display()))?
    {
        let entry = entry?;
        let ft = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path).with_context(|| {
                format!(
                    "Failed to copy {} to {}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }
    Ok(())
}

async fn cmd_import(_config: &AppConfig, path: &Path, force: bool) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("Backup directory does not exist: {}", path.display());
    }
    if !path.is_dir() {
        anyhow::bail!("Path is not a directory: {}", path.display());
    }

    let config_backup = path.join("config");
    let data_backup = path.join("data");

    if !config_backup.exists() && !data_backup.exists() {
        anyhow::bail!(
            "No Operant backup found at {}. Expected 'config/' or 'data/' subdirectories.",
            path.display()
        );
    }

    if !force {
        eprintln!("This will overwrite existing files in:");
        if config_backup.exists() {
            eprintln!(
                "  Config: {}",
                operant_core::platform::operant_config_dir().display()
            );
        }
        if data_backup.exists() {
            eprintln!(
                "  Data:   {}",
                operant_core::platform::operant_data_dir().display()
            );
        }
        eprintln!("Use --force to skip this warning.");
        return Ok(());
    }

    let mut restored = 0u32;
    if config_backup.exists() {
        let dst = operant_core::platform::operant_config_dir();
        copy_dir_recursive(&config_backup, &dst)?;
        println!("Restored config from {}", config_backup.display());
        restored += 1;
    }
    if data_backup.exists() {
        let dst = operant_core::platform::operant_data_dir();
        copy_dir_recursive(&data_backup, &dst)?;
        println!("Restored data from {}", data_backup.display());
        restored += 1;
    }

    println!("Import complete. {} directories restored.", restored);
    Ok(())
}
