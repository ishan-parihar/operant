use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Subcommand;
use hermes_core::config::AppConfig;

#[derive(Debug, Clone, Subcommand)]
pub enum LogsSubcommand {
    /// Show recent log entries
    Show {
        /// Number of lines to show
        #[arg(default_value = "50")]
        lines: usize,
        /// Filter by log level (e.g. INFO, WARN, ERROR)
        #[arg(long)]
        level: Option<String>,
        /// Filter by component
        #[arg(long)]
        component: Option<String>,
    },
    /// Follow log output (tail -f style)
    Follow {
        /// Filter by log level
        #[arg(long)]
        level: Option<String>,
    },
}

pub async fn handle_logs_command(config: &AppConfig, cmd: LogsSubcommand) -> Result<()> {
    match cmd {
        LogsSubcommand::Show {
            lines,
            level,
            component,
        } => cmd_show(config, lines, level.as_deref(), component.as_deref()).await,
        LogsSubcommand::Follow { level } => cmd_follow(config, level.as_deref()).await,
    }
}

fn log_path(config: &AppConfig) -> Option<PathBuf> {
    if let Some(path) = &config.logging.log_file {
        return Some(PathBuf::from(path));
    }
    let default = hermes_core::platform::hermes_data_dir().join("hermes.log");
    if default.exists() {
        return Some(default);
    }
    None
}

async fn cmd_show(
    config: &AppConfig,
    lines: usize,
    level: Option<&str>,
    component: Option<&str>,
) -> Result<()> {
    let path = log_path(config)
        .context("No log file found. Configure logging.log_file or run Hermes first.")?;
    let file =
        fs::File::open(&path).with_context(|| format!("Failed to open log: {}", path.display()))?;
    let reader = BufReader::new(file);
    let all_lines: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();
    let slice: Vec<&String> = all_lines
        .iter()
        .filter(|l| {
            let level_ok = level.map_or(true, |lv| l.contains(lv));
            let comp_ok = component.map_or(true, |c| l.contains(c));
            level_ok && comp_ok
        })
        .collect();
    let start = if slice.len() > lines {
        slice.len() - lines
    } else {
        0
    };
    for line in &slice[start..] {
        println!("{}", line);
    }
    Ok(())
}

async fn cmd_follow(config: &AppConfig, level: Option<&str>) -> Result<()> {
    let path = log_path(config).context("No log file found.")?;
    let mut last_size = fs::metadata(&path)?.len();
    println!("Following: {} (Ctrl+C to stop)", path.display());
    loop {
        let current_size = fs::metadata(&path)?.len();
        if current_size > last_size {
            let file = fs::File::open(&path)?;
            let reader = BufReader::new(file);
            let lines = reader.lines();
            // Seek to where we were
            let mut file2 = fs::File::open(&path)?;
            file2.seek(SeekFrom::Start(last_size))?;
            let reader2 = BufReader::new(file2);
            for line in reader2.lines().filter_map(|l| l.ok()) {
                if level.map_or(true, |lv| line.contains(lv)) {
                    println!("{}", line);
                }
            }
            last_size = current_size;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}
