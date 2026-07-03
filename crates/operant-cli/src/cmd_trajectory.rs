//! CLI subcommand for managing agent trajectories.
//!
//! Trajectories are saved by the agent loop (when enabled via
//! `AgentConfig::record_trajectories` or `--record-trajectory`) to
//! `~/.operant/trajectories/<session_id>-<timestamp>.json`.
//!
//! This subcommand lists saved trajectories and exports them as JSON,
//! NDJSON, or prompt-completion pairs for fine-tuning.

use anyhow::{Context, Result};
use clap::Subcommand;
use console::style;
use operant_core::trajectory::{Trajectory, TrajectoryExporter};
use std::path::PathBuf;

/// Manage agent trajectories (ReAct step recordings for fine-tuning/analysis)
#[derive(Debug, Clone, Subcommand)]
pub enum TrajectorySubcommand {
    /// List all saved trajectories
    List,
    /// Show details of a specific trajectory
    Show {
        /// Trajectory ID (filename without .json)
        id: String,
    },
    /// Export all trajectories as a single JSON array
    Export {
        /// Output format: json, ndjson
        #[arg(long, default_value = "json")]
        format: String,
        /// Output file (defaults to stdout)
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Delete a trajectory
    Delete {
        /// Trajectory ID
        id: String,
    },
    /// Delete all trajectories
    Clear,
}

/// Handle a trajectory subcommand.
pub async fn handle_trajectory_command(cmd: TrajectorySubcommand) -> Result<()> {
    let trajectories_dir = operant_core::platform::operant_home().join("trajectories");

    match cmd {
        TrajectorySubcommand::List => list_trajectories(&trajectories_dir),
        TrajectorySubcommand::Show { id } => show_trajectory(&trajectories_dir, &id),
        TrajectorySubcommand::Export { format, output } => {
            export_trajectories(&trajectories_dir, &format, output)
        }
        TrajectorySubcommand::Delete { id } => delete_trajectory(&trajectories_dir, &id),
        TrajectorySubcommand::Clear => clear_trajectories(&trajectories_dir),
    }
}

fn list_trajectories(dir: &std::path::Path) -> Result<()> {
    if !dir.exists() {
        println!("No trajectories directory at {}.", dir.display());
        println!("Trajectories are saved when the agent runs with recording enabled.");
        return Ok(());
    }

    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("Failed to read {}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();

    entries.sort_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()));

    if entries.is_empty() {
        println!("No trajectories saved yet.");
        println!();
        println!("Enable trajectory recording with:");
        println!("  operant run --query '...' --record-trajectory");
        return Ok(());
    }

    println!("{} ({} total)", style("Saved Trajectories").bold(), entries.len());
    println!("{}", "─".repeat(80));
    println!(
        "{:<40} {:<12} {:<10} {:<8}",
        "ID", "Model", "Steps", "Tools"
    );
    println!("{}", "─".repeat(80));

    for entry in &entries {
        let path = entry.path();
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        let json = match std::fs::read_to_string(&path) {
            Ok(j) => j,
            Err(_) => continue,
        };
        let traj: Trajectory = match serde_json::from_str(&json) {
            Ok(t) => t,
            Err(_) => continue,
        };
        println!(
            "{:<40} {:<12} {:<10} {:<8}",
            id, traj.model, traj.steps.len(), traj.tool_calls
        );
    }

    Ok(())
}

fn show_trajectory(dir: &std::path::Path, id: &str) -> Result<()> {
    let path = dir.join(format!("{}.json", id));
    if !path.exists() {
        anyhow::bail!("Trajectory '{}' not found at {}", id, path.display());
    }

    let json = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let traj: Trajectory = serde_json::from_str(&json).context("Failed to parse trajectory JSON")?;

    println!("{}", style("Trajectory Details").bold());
    println!("{}", "─".repeat(60));
    println!("  ID:          {}", traj.id);
    println!("  Session:     {}", traj.session_id);
    println!("  Model:       {}", traj.model);
    println!("  Timestamp:   {}", traj.timestamp);
    println!("  Iterations:  {}", traj.iterations);
    println!("  Tool calls:  {}", traj.tool_calls);
    println!("  Tokens:      {}", traj.total_tokens);
    println!("  Success:     {}", traj.success);
    println!("  Steps:       {}", traj.steps.len());
    println!("{}", "─".repeat(60));
    println!();

    for step in &traj.steps {
        println!("{}", style(format!("Step {}", step.step)).cyan().bold());
        if let Some(ref thought) = step.thought {
            println!("  Thought:    {}", truncate(thought, 100));
        }
        if let Some(ref action) = step.action {
            println!("  Action:     {}", action);
        }
        if let Some(ref args) = step.action_args {
            println!("  Args:       {}", truncate(args, 100));
        }
        if let Some(ref obs) = step.observation {
            println!("  Observation: {}", truncate(obs, 100));
        }
        if let Some(ref resp) = step.response {
            println!("  Response:   {}", truncate(resp, 200));
        }
        println!();
    }

    Ok(())
}

fn export_trajectories(
    dir: &std::path::Path,
    format: &str,
    output: Option<PathBuf>,
) -> Result<()> {
    if !dir.exists() {
        anyhow::bail!("No trajectories directory at {}", dir.display());
    }

    let mut exporter = TrajectoryExporter::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let json = std::fs::read_to_string(entry.path())?;
        if let Ok(traj) = serde_json::from_str::<Trajectory>(&json) {
            exporter.add(traj);
        }
    }

    if exporter.is_empty() {
        println!("No trajectories to export.");
        return Ok(());
    }

    let content = match format {
        "json" => exporter.export_json().context("Failed to export JSON")?,
        "ndjson" => exporter.export_ndjson(),
        other => anyhow::bail!("Unknown format: {} (use json or ndjson)", other),
    };

    match output {
        Some(path) => {
            std::fs::write(&path, &content)
                .with_context(|| format!("Failed to write {}", path.display()))?;
            println!(
                "{} Exported {} trajectories to {}",
                style("✓").green(),
                exporter.len(),
                path.display()
            );
        }
        None => {
            print!("{}", content);
        }
    }

    Ok(())
}

fn delete_trajectory(dir: &std::path::Path, id: &str) -> Result<()> {
    let path = dir.join(format!("{}.json", id));
    if !path.exists() {
        anyhow::bail!("Trajectory '{}' not found", id);
    }
    std::fs::remove_file(&path).with_context(|| format!("Failed to delete {}", path.display()))?;
    println!("{} Trajectory '{}' deleted.", style("✓").green(), id);
    Ok(())
}

fn clear_trajectories(dir: &std::path::Path) -> Result<()> {
    if !dir.exists() {
        println!("No trajectories directory.");
        return Ok(());
    }
    let mut count = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
            let _ = std::fs::remove_file(entry.path());
            count += 1;
        }
    }
    println!("{} Cleared {} trajectory file(s).", style("✓").green(), count);
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
