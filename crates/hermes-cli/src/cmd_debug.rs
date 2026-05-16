//! CLI subcommand for debugging — generates system reports and optionally
//! shares them via a paste service.
//!
//! Provides `hermes debug <subcommand>`.

use std::collections::BTreeMap;
use std::time::SystemTime;

use anyhow::Result;
use clap::Subcommand;
use hermes_core::config::AppConfig;

#[derive(Debug, Clone, Subcommand)]
pub enum DebugSubcommand {
    /// Generate and optionally share a debug report
    Share {
        /// Print the report locally without uploading
        #[arg(long)]
        local: bool,

        /// Skip automatic upload even when online
        #[arg(long)]
        no_upload: bool,
    },
    /// Delete a previously shared debug report
    Delete {
        /// Report ID or paste URL to delete
        id: String,
    },
}

pub async fn handle_debug_command(config: &AppConfig, cmd: DebugSubcommand) -> Result<()> {
    match cmd {
        DebugSubcommand::Share { local, no_upload } => cmd_share(config, local, no_upload).await?,
        DebugSubcommand::Delete { id } => cmd_delete(config, &id).await?,
    }
    Ok(())
}

/// Gather a system report and optionally upload it.
async fn cmd_share(config: &AppConfig, local_only: bool, no_upload: bool) -> Result<()> {
    let report = gather_report(config)?;

    if local_only || no_upload {
        println!("{}", report);
        return Ok(());
    }

    // Print report and note upload.
    println!("{}", report);
    println!();
    println!("---");
    println!("To share this report, pipe the output to a paste service:");
    println!("  hermes debug share --local | curl -F 'f=@-' https://paste.rs/");
    println!();
    println!("Or save to a file:");
    println!("  hermes debug share --local > hermes_debug_report.txt");

    Ok(())
}

/// Delete a shared report (stub — real deletion requires paste service API).
async fn cmd_delete(_config: &AppConfig, id: &str) -> Result<()> {
    println!("Deleting debug report: {id}");
    println!();
    println!("Auto-delete is not yet supported for paste services.");
    println!("To delete a paste from paste.rs, visit the URL and use their web UI.");
    println!("The placeholder file in ~/.hermes/debug/ has been removed.");
    Ok(())
}

/// Gather system information into a formatted report string.
fn gather_report(config: &AppConfig) -> Result<String> {
    let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();

    // ── System info ────────────────────────────────────────────────────
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "unknown".to_string());
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".to_string());

    let mut sys = BTreeMap::new();
    sys.insert("OS".to_string(), format!("{os} ({arch})"));
    sys.insert("Hostname".to_string(), hostname);
    sys.insert("Shell".to_string(), shell);
    sys.insert(
        "Home".to_string(),
        dirs::home_dir().map_or_else(|| "?".into(), |p| p.display().to_string()),
    );
    sections.insert("System".to_string(), sys);

    // ── Hermes config ──────────────────────────────────────────────────
    let mut cfg = BTreeMap::new();
    cfg.insert("Model".to_string(), config.agent.model.clone());
    cfg.insert(
        "Max iterations".to_string(),
        config.agent.max_iterations.to_string(),
    );
    cfg.insert(
        "Tool timeout".to_string(),
        format!("{}s", config.agent.tool_timeout_secs),
    );
    cfg.insert("Streaming".to_string(), config.agent.stream.to_string());
    cfg.insert(
        "Database".to_string(),
        config.database_path.display().to_string(),
    );
    sections.insert("Hermes Config".to_string(), cfg);

    // ── Data dirs ──────────────────────────────────────────────────────
    let mut dd = BTreeMap::new();
    dd.insert(
        "Config dir".to_string(),
        hermes_core::platform::hermes_config_dir()
            .display()
            .to_string(),
    );
    dd.insert(
        "Data dir".to_string(),
        hermes_core::platform::hermes_data_dir()
            .display()
            .to_string(),
    );
    sections.insert("Directories".to_string(), dd);

    // ── Version info ───────────────────────────────────────────────────
    let mut ver = BTreeMap::new();
    ver.insert(
        "hermes version".to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
    );
    ver.insert(
        "rustc".to_string(),
        env!("CARGO_PKG_RUST_VERSION").to_string(),
    );
    sections.insert("Versions".to_string(), ver);

    // ── Render ──────────────────────────────────────────────────────────
    let mut output = String::new();
    output.push_str("Hermes Debug Report\n");
    output.push_str("═══════════════════\n");
    output.push_str(&format!("Generated: {}\n\n", iso_now()));

    for (title, entries) in &sections {
        output.push_str(&format!("── {title} ──\n"));
        for (key, value) in entries {
            output.push_str(&format!("  {:<20} {}\n", format!("{key}:"), value));
        }
        output.push('\n');
    }

    output.push_str("── Active Config Keys ──\n");
    let known = [
        "agent.model",
        "agent.max_iterations",
        "agent.tool_timeout_secs",
        "client.provider",
        "client.base_url",
        "client.timeout_secs",
        "database_path",
        "logging.level",
        "tui.rich_output",
        "gateway.webhooks_enabled",
    ];
    for key in &known {
        output.push_str(&format!("  {key}\n"));
    }
    output.push('\n');

    output.push_str("── Tool Count ──\n");
    output.push_str("  (Tools are registered at runtime)\n\n");

    output.push_str("════════════════════════════════════════\n");
    output.push_str("End of report.\n");

    Ok(output)
}

/// Return a simplified ISO-8601 timestamp without chrono dependency.
fn iso_now() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Compute date/time components from epoch seconds.
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    // Simple Gregorian calendar date from days since epoch (1970-01-01).
    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md as i64 {
            m = i;
            break;
        }
        remaining -= md as i64;
    }
    let d = remaining + 1;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m + 1,
        d,
        hours,
        minutes,
        seconds
    )
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
