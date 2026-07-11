use anyhow::{Context, Result};
use clap::Subcommand;
use console::style;
use dialoguer::Confirm;
use operant_core::config::AppConfig;
use operant_core::skills::SkillManager;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tempfile as _;

/// Manage installed skills.
#[derive(Debug, Clone, Subcommand)]
pub enum SkillsSubcommand {
    /// List all installed skills
    List,
    /// Search available skills by name or description
    Search {
        /// Search query (matched case-insensitively against name and description)
        query: String,
    },
    /// Show detailed information about a specific skill
    Inspect {
        /// Skill name to inspect
        id: String,
    },
    /// Install a skill from a local file or URL.
    ///
    /// Runs a pre-install security scan (skills_guard) and blocks
    /// installation if high-severity threats are found. Use --force to
    /// override the scan verdict (not recommended for untrusted sources).
    Install {
        /// Source path or URL to the skill content
        source: String,
        /// Optional name override (defaults to the file stem or URL basename)
        #[arg(long)]
        name: Option<String>,
        /// Skip the security scan and install regardless of findings
        #[arg(long)]
        force: bool,
    },
    /// Uninstall a skill (removes its directory)
    Uninstall {
        /// Name of the skill to remove
        name: String,
    },
    /// Re-read a skill's SKILL.md to refresh in-memory state
    Update {
        /// Name of the skill to update
        name: String,
    },
    /// List all skill directories and show installed status
    Browse,
    /// Check if a skill's prerequisites are met
    Check {
        /// Name of the skill to check
        name: String,
    },
    /// Audit all installed skills for validity and completeness
    Audit,
    /// Delete and re-create a skill from its in-memory content
    Reset {
        /// Name of the skill to reset
        name: String,
    },
    /// Create a skill manifest JSON for sharing
    Publish {
        /// Skill name
        name: String,
        /// Manifest description
        description: String,
        /// Manifest version
        version: String,
    },
    /// Export a skill as a standalone archive (tar.gz)
    Snapshot {
        /// Name of the skill to snapshot
        name: String,
        /// Output file path (defaults to {name}.tar.gz)
        #[arg(long)]
        output: Option<String>,
    },
    /// Manage external skill sources (taps)
    Tap {
        #[command(subcommand)]
        command: TapCommand,
    },
    /// Enable or disable a skill by toggling SKILL.md
    Toggle {
        /// Name of the skill to toggle
        name: String,
    },
    /// Search the remote skill marketplace registry
    Market {
        #[command(subcommand)]
        command: MarketCommand,
    },
}

/// Marketplace subcommands (iter-133 — closes the ponytail-audit gap
/// "skill marketplace missing").
#[derive(Debug, Clone, Subcommand)]
pub enum MarketCommand {
    /// Search the registry by name, description, or tag
    Search { query: String },
    /// Install a skill from the registry by exact name
    Install {
        name: String,
        #[arg(long)]
        force: bool,
    },
    /// List all skills in the registry
    List,
    /// Force-refresh the cached registry index
    Refresh,
    /// Check if any installed skills have updates available
    Updates,
}

/// Manage external skill sources (taps).
#[derive(Debug, Clone, Subcommand)]
pub enum TapCommand {
    /// List all configured taps
    List,
    /// Add a new tap source
    Add {
        /// Source URL of the tap
        source: String,
        /// Optional short alias for this tap
        #[arg(long)]
        alias: Option<String>,
    },
    /// Remove a tap by name or alias
    Remove {
        /// Name or alias of the tap to remove
        name: String,
    },
}

pub async fn handle_skills_command(
    config: &AppConfig,
    cmd: SkillsSubcommand,
    json: bool,
) -> Result<()> {
    match cmd {
        SkillsSubcommand::List => list_skills(config, json),
        SkillsSubcommand::Search { query } => search_skills(config, &query),
        SkillsSubcommand::Inspect { id } => inspect_skill(config, &id),
        SkillsSubcommand::Install {
            source,
            name,
            force,
        } => install_skill(config, &source, name.as_deref(), force).await,
        SkillsSubcommand::Uninstall { name } => uninstall_skill(config, &name),
        SkillsSubcommand::Update { name } => update_skill(config, &name),
        SkillsSubcommand::Browse => browse_skills(config),
        SkillsSubcommand::Check { name } => check_skill(config, &name),
        SkillsSubcommand::Audit => audit_skills(config),
        SkillsSubcommand::Reset { name } => reset_skill(config, &name),
        SkillsSubcommand::Publish {
            name,
            description,
            version,
        } => publish_skill(config, &name, &description, &version),
        SkillsSubcommand::Snapshot { name, output } => {
            snapshot_skill(config, &name, output.as_deref())
        }
        SkillsSubcommand::Tap { command } => handle_tap_command(config, command),
        SkillsSubcommand::Toggle { name } => toggle_skill(config, &name),
        SkillsSubcommand::Market { command } => handle_market_command(config, command).await,
    }
}

// ---------------------------------------------------------------------------
// Marketplace handlers (iter-133)
// ---------------------------------------------------------------------------

async fn handle_market_command(_config: &AppConfig, cmd: MarketCommand) -> Result<()> {
    let marketplace = operant_core::skill_marketplace::SkillMarketplace::new();
    match cmd {
        MarketCommand::Search { query } => {
            let entries = marketplace.search(&query).await?;
            if entries.is_empty() {
                println!("No skills found matching '{}'.", style(&query).cyan());
                return Ok(());
            }
            println!("Found {} skill(s):\n", entries.len());
            for e in &entries {
                println!(
                    "  {} v{} — {}",
                    style(&e.name).green().bold(),
                    style(&e.version).dim(),
                    e.description
                );
            }
        }
        MarketCommand::Install { name, force } => {
            let skills_dir = operant_core::platform::operant_skills_dir();
            match marketplace.install(&name, &skills_dir, force).await {
                Ok(path) => println!(
                    "{} Installed '{}' to {}",
                    style("✓").green(),
                    style(&name).bold(),
                    path.display()
                ),
                Err(e) => anyhow::bail!("Install failed: {e}"),
            }
        }
        MarketCommand::List => {
            let entries = marketplace.fetch_index().await?;
            println!("{} skill(s) available:\n", entries.len());
            for e in &entries {
                println!(
                    "  {} v{} — {}",
                    style(&e.name).green().bold(),
                    style(&e.version).dim(),
                    e.description
                );
            }
        }
        MarketCommand::Refresh => {
            let entries = marketplace.refresh_index().await?;
            println!(
                "{} Refreshed — {} skill(s)",
                style("✓").green(),
                entries.len()
            );
        }
        MarketCommand::Updates => {
            let skills_dir = operant_core::platform::operant_skills_dir();
            let mut mgr = SkillManager::new(skills_dir);
            let installed = mgr.load_all().unwrap_or_default();
            if installed.is_empty() {
                println!("No installed skills to check.");
                return Ok(());
            }
            for skill in &installed {
                match marketplace
                    .check_for_update(&skill.name, &skill.version)
                    .await
                {
                    Ok(Some(entry)) => println!(
                        "  {} v{} → v{}",
                        style(&skill.name).green().bold(),
                        skill.version,
                        entry.version
                    ),
                    Ok(None) => println!(
                        "  {} v{} (up to date)",
                        style(&skill.name).dim(),
                        skill.version
                    ),
                    Err(e) => println!("  {} (check failed: {e})", style(&skill.name).yellow()),
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn list_skills(config: &AppConfig, json: bool) -> Result<()> {
    let skills_dir = &config.skills.root_dir;

    if !skills_dir.exists() {
        if json {
            println!("[]");
        } else {
            println!("No skills installed.");
        }
        return Ok(());
    }

    let mut manager = SkillManager::new(skills_dir.clone());
    manager
        .load_all()
        .with_context(|| format!("Failed to load skills from '{}'", skills_dir.display()))?;

    let skills = manager.list();

    if json {
        let items: Vec<serde_json::Value> = skills
            .iter()
            .map(|(name, desc)| serde_json::json!({"name": name, "description": desc}))
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    if skills.is_empty() {
        println!("No skills installed.");
        return Ok(());
    }

    println!("Installed skills ({}):", skills.len());
    println!();
    for (name, description) in &skills {
        println!("  {:<24} {}", name, description);
    }

    Ok(())
}

fn search_skills(config: &AppConfig, query: &str) -> Result<()> {
    let skills_dir = &config.skills.root_dir;

    if !skills_dir.exists() {
        println!("No skills found matching '{}'.", query);
        return Ok(());
    }

    let mut manager = SkillManager::new(skills_dir.clone());
    let all = manager
        .load_all()
        .with_context(|| format!("Failed to load skills from '{}'", skills_dir.display()))?;

    let query_lower = query.to_lowercase();
    let matches: Vec<_> = all
        .iter()
        .filter(|s| {
            s.name.to_lowercase().contains(&query_lower)
                || s.description.to_lowercase().contains(&query_lower)
        })
        .collect();

    if matches.is_empty() {
        println!("No skills found matching '{}'.", query);
        return Ok(());
    }

    println!("Skills matching '{}' ({}):", query, matches.len());
    println!();
    for skill in &matches {
        println!("  {:<24} {}", skill.name, skill.description);
    }

    Ok(())
}

fn inspect_skill(config: &AppConfig, id: &str) -> Result<()> {
    let skills_dir = &config.skills.root_dir;

    if !skills_dir.exists() {
        anyhow::bail!("Skill '{}' not found.", id);
    }

    let mut manager = SkillManager::new(skills_dir.clone());
    manager
        .load_all()
        .with_context(|| format!("Failed to load skills from '{}'", skills_dir.display()))?;

    let skill = match manager.get(id) {
        Some(s) => s,
        None => anyhow::bail!("Skill '{}' not found.", id),
    };

    println!("Name:        {}", skill.name);
    println!("Description: {}", skill.description);
    println!("Version:     {}", skill.version);

    if !skill.platforms.is_empty() {
        println!("Platforms:   {}", skill.platforms.join(", "));
    }
    if !skill.prerequisites_env.is_empty() {
        println!("Env vars:    {}", skill.prerequisites_env.join(", "));
    }
    if !skill.prerequisites_commands.is_empty() {
        println!("Commands:    {}", skill.prerequisites_commands.join(", "));
    }
    if !skill.references.is_empty() {
        let ref_names: Vec<&str> = skill.references.keys().map(String::as_str).collect();
        println!("References:  {}", ref_names.join(", "));
    }

    if !skill.content.is_empty() {
        println!();
        println!("--- Content ---");
        println!("{}", skill.content);
    }

    Ok(())
}

/// Install a skill from a local file or URL.
///
/// Before writing the skill to the skills directory, runs a pre-install
/// security scan via `operant_core::skills_guard`. The scan checks for:
///   - prompt injection patterns
///   - shell injection / reverse shell patterns
///   - credential exfiltration patterns
///   - suspicious network calls
///   - known malicious patterns
///
/// If the scan verdict is Block (high-severity findings), installation is
/// refused unless `--force` is passed. If the verdict is Confirm (medium-
/// severity findings), the user is prompted to confirm.
async fn install_skill(
    config: &AppConfig,
    source: &str,
    name: Option<&str>,
    force: bool,
) -> Result<()> {
    let (content, skill_name) = if source.starts_with("http://") || source.starts_with("https://") {
        let response = reqwest::get(source)
            .await
            .with_context(|| format!("Failed to download '{}'", source))?;

        if !response.status().is_success() {
            anyhow::bail!("Download failed with status {}", response.status());
        }

        let body = response
            .text()
            .await
            .context("Failed to read response body")?;

        let derived = name
            .map(|s| s.to_string())
            .unwrap_or_else(|| derive_name_from_url(source));

        (body, derived)
    } else {
        let path = Path::new(source);
        if !path.exists() {
            anyhow::bail!("Source file '{}' not found.", source);
        }

        let body = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read '{}'", source))?;

        let derived = name.map(|s| s.to_string()).unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("skill")
                .to_string()
        });

        (body, derived)
    };

    // ── Pre-install security scan ──
    // Write the content to a temp file so skills_guard can scan it as a file.
    let temp_dir = tempfile::tempdir().context("Failed to create temp dir for scan")?;
    let temp_skill_path = temp_dir.path().join(format!("{}.md", skill_name));
    std::fs::write(&temp_skill_path, &content)
        .with_context(|| "Failed to write skill content to temp file for scanning")?;

    let scan_result = operant_core::skills_guard::scan_skill(&temp_skill_path, source);
    let (allow, reason) = operant_core::skills_guard::should_allow_install(&scan_result, force);

    // Always print the scan summary so the user knows what was found.
    if !scan_result.findings.is_empty() {
        println!(
            "{}",
            operant_core::skills_guard::format_scan_report(&scan_result)
        );
    }

    match allow {
        Some(true) => {
            if !scan_result.findings.is_empty() {
                println!("{} {}", style("⚠").yellow(), reason);
            }
        }
        Some(false) => {
            anyhow::bail!(
                "Installation blocked by security scan: {}\n\
                 {} findings ({} high, {} medium, {} low).\n\
                 To install anyway, re-run with --force.",
                reason,
                scan_result.findings.len(),
                scan_result
                    .findings
                    .iter()
                    .filter(|f| f.severity == operant_core::skills_guard::Severity::High)
                    .count(),
                scan_result
                    .findings
                    .iter()
                    .filter(|f| f.severity == operant_core::skills_guard::Severity::Medium)
                    .count(),
                scan_result
                    .findings
                    .iter()
                    .filter(|f| f.severity == operant_core::skills_guard::Severity::Low)
                    .count(),
            );
        }
        None => {
            // Confirm: prompt the user
            println!(
                "{}",
                style("⚠ Security scan requires confirmation:").yellow()
            );
            println!("  {}", reason);
            if !Confirm::new()
                .with_prompt(format!("Install skill '{}' anyway?", skill_name))
                .default(false)
                .interact()
                .context("Failed to read confirmation")?
            {
                println!("Installation cancelled.");
                return Ok(());
            }
        }
    }

    let mut manager = SkillManager::new(config.skills.root_dir.clone());
    manager
        .create(&skill_name, &content)
        .with_context(|| format!("Failed to install skill '{}'", skill_name))?;

    println!(
        "{} Skill '{}' installed successfully.",
        style("✓").green(),
        skill_name
    );
    Ok(())
}

fn uninstall_skill(config: &AppConfig, name: &str) -> Result<()> {
    if !Confirm::new()
        .with_prompt(format!(
            "Are you sure you want to uninstall skill '{}'?",
            name
        ))
        .interact()
        .context("Failed to read confirmation")?
    {
        println!("Uninstall cancelled.");
        return Ok(());
    }

    let mut manager = SkillManager::new(config.skills.root_dir.clone());
    manager
        .delete(name)
        .with_context(|| format!("Failed to uninstall skill '{}'", name))?;

    println!("{} Skill '{}' uninstalled.", style("✓").green(), name);
    Ok(())
}

fn update_skill(config: &AppConfig, name: &str) -> Result<()> {
    let skills_dir = &config.skills.root_dir;
    let mut manager = SkillManager::new(skills_dir.clone());
    manager.load_all().context("Failed to load skills")?;

    let old_skill = manager
        .get(name)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Skill '{}' not found.", name))?;

    // Reload from disk to pick up any changes
    manager.load_all().context("Failed to reload skills")?;

    let updated = manager
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("Skill '{}' not found after reload.", name))?;

    println!("{} Skill '{}' updated.", style("✓").green(), name);
    if updated.version != old_skill.version {
        println!(
            "  Version changed: {} → {}",
            old_skill.version, updated.version
        );
    }

    Ok(())
}

fn browse_skills(config: &AppConfig) -> Result<()> {
    let skills_dir = &config.skills.root_dir;

    if !skills_dir.exists() {
        println!("Skills directory not found at: {}", skills_dir.display());
        return Ok(());
    }

    let mut manager = SkillManager::new(skills_dir.clone());
    let installed = manager.load_all().context("Failed to load skills")?;

    let entries = match std::fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(_) => {
            println!("Could not read skills directory.");
            return Ok(());
        }
    };

    let mut found = false;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let dir_name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden directories
        if dir_name.starts_with('.') {
            continue;
        }

        let has_skill = path.join("SKILL.md").exists();
        let is_disabled = path.join("SKILL.md.disabled").exists();
        let is_loaded = installed.iter().any(|s| s.name == dir_name);

        let status = if is_loaded && has_skill {
            style("✓ installed").green()
        } else if is_disabled {
            style("  disabled").yellow()
        } else if has_skill {
            style("  available").yellow()
        } else {
            style("  no SKILL.md").dim()
        };

        println!("  {:<24} {}", dir_name, status);
        found = true;
    }

    if !found {
        println!("No skill directories found in '{}'.", skills_dir.display());
    }

    Ok(())
}

fn check_skill(config: &AppConfig, name: &str) -> Result<()> {
    let mut manager = SkillManager::new(config.skills.root_dir.clone());
    manager.load_all().context("Failed to load skills")?;

    let skill = manager
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("Skill '{}' not found.", name))?;

    println!("Checking skill '{}' ...", name);
    println!();

    // Platform check
    if !skill.platforms.is_empty() {
        let current = current_platform();
        if skill.platforms.iter().any(|p| p == current) {
            println!(
                "  {} Platform: compatible ({})",
                style("✓").green(),
                current
            );
        } else {
            println!(
                "  {} Platform: incompatible (requires {})",
                style("✗").red(),
                skill.platforms.join(", ")
            );
        }
    } else {
        println!("  {} Platform: any (no restrictions)", style("✓").green());
    }

    // Environment variable checks
    if skill.prerequisites_env.is_empty() {
        println!("  {} Env vars: none required", style("✓").green());
    } else {
        for var in &skill.prerequisites_env {
            if std::env::var(var).is_ok() {
                println!("  {} Env var '{}': set", style("✓").green(), var);
            } else {
                println!("  {} Env var '{}': NOT set", style("✗").red(), var);
            }
        }
    }

    // Command checks
    if skill.prerequisites_commands.is_empty() {
        println!("  {} Commands: none required", style("✓").green());
    } else {
        for cmd in &skill.prerequisites_commands {
            if which::which(cmd).is_ok() {
                println!("  {} Command '{}': found", style("✓").green(), cmd);
            } else {
                println!("  {} Command '{}': NOT found", style("✗").red(), cmd);
            }
        }
    }

    let available = manager.is_available(skill);
    println!();
    if available {
        println!(
            "{} Skill '{}' is fully available.",
            style("✓").green(),
            name
        );
    } else {
        println!(
            "{} Skill '{}' has unmet prerequisites.",
            style("⚠").yellow(),
            name
        );
    }

    Ok(())
}

fn audit_skills(config: &AppConfig) -> Result<()> {
    let skills_dir = &config.skills.root_dir;

    if !skills_dir.exists() {
        println!("No skills directory found.");
        return Ok(());
    }

    let mut manager = SkillManager::new(skills_dir.clone());
    manager.load_all().context("Failed to load skills")?;

    let entries = match std::fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(_) => {
            println!("Could not read skills directory.");
            return Ok(());
        }
    };

    println!("Skill Audit Report");
    println!("{}", "=".repeat(60));
    println!();

    let mut total_issues: usize = 0;
    let mut skill_count: usize = 0;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let dir_name = entry.file_name().to_string_lossy().to_string();
        if dir_name.starts_with('.') {
            continue;
        }

        skill_count += 1;
        let skill_path = path.join("SKILL.md");
        let disabled_path = path.join("SKILL.md.disabled");

        print!("{}: ", dir_name);

        // Disabled skills are noted but not audited in depth
        if disabled_path.exists() && !skill_path.exists() {
            println!("{}", style("DISABLED").yellow());
            continue;
        }

        let mut issues: Vec<String> = Vec::new();

        if !skill_path.exists() {
            issues.push("Missing SKILL.md file".to_string());
        }

        let skill = manager.get(&dir_name);

        match skill {
            None => {
                issues.push("Failed to parse SKILL.md".to_string());
            }
            Some(s) => {
                for var in &s.prerequisites_env {
                    if std::env::var(var).is_err() {
                        issues.push(format!("Missing env var: {}", var));
                    }
                }
                for cmd in &s.prerequisites_commands {
                    if !which::which(cmd).is_ok() {
                        issues.push(format!("Missing command: {}", cmd));
                    }
                }
                if !s.platforms.is_empty() {
                    let current = current_platform();
                    if !s.platforms.iter().any(|p| p == current) {
                        issues.push(format!(
                            "Platform mismatch: requires {}",
                            s.platforms.join(", ")
                        ));
                    }
                }
            }
        }

        if issues.is_empty() {
            println!("{}", style("OK").green());
        } else {
            total_issues += issues.len();
            println!(
                "{} ({} issue{})",
                style("ISSUES").red(),
                issues.len(),
                if issues.len() == 1 { "" } else { "s" }
            );
            for issue in &issues {
                println!("    - {}", style(issue).red());
            }
        }
    }

    if skill_count == 0 {
        println!("No skill directories found.");
    }

    println!();
    if total_issues == 0 {
        println!(
            "{} No issues found across {} skill(s).",
            style("✓").green(),
            skill_count
        );
    } else {
        println!(
            "{} {} issue(s) found across {} skill(s).",
            style("⚠").yellow(),
            total_issues,
            skill_count
        );
    }

    Ok(())
}

fn reset_skill(config: &AppConfig, name: &str) -> Result<()> {
    let mut manager = SkillManager::new(config.skills.root_dir.clone());
    manager.load_all().context("Failed to load skills")?;

    let skill = manager
        .get(name)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Skill '{}' not found.", name))?;

    if !Confirm::new()
        .with_prompt(format!(
            "Reset skill '{}' to its original content? This will delete and re-create the skill.",
            name
        ))
        .interact()
        .context("Failed to read confirmation")?
    {
        println!("Reset cancelled.");
        return Ok(());
    }

    // Reconstruct the SKILL.md content
    let content = reconstruct_skill_md(&skill);

    manager
        .delete(name)
        .with_context(|| format!("Failed to delete skill '{}'", name))?;

    manager
        .create(name, &content)
        .with_context(|| format!("Failed to re-create skill '{}'", name))?;

    println!("{} Skill '{}' has been reset.", style("✓").green(), name);
    Ok(())
}

fn publish_skill(config: &AppConfig, name: &str, description: &str, version: &str) -> Result<()> {
    let mut manager = SkillManager::new(config.skills.root_dir.clone());
    manager.load_all().context("Failed to load skills")?;

    let skill = manager
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("Skill '{}' not found.", name))?;

    let manifest = serde_json::json!({
        "name": skill.name,
        "description": description,
        "version": version,
        "original_version": skill.version,
        "platforms": skill.platforms,
        "prerequisites_env": skill.prerequisites_env,
        "prerequisites_commands": skill.prerequisites_commands,
        "content": skill.content,
        "references": skill.references,
    });

    let manifest_path = config
        .skills
        .root_dir
        .join(format!("{}.manifest.json", name));

    let json = serde_json::to_string_pretty(&manifest).context("Failed to serialize manifest")?;

    std::fs::write(&manifest_path, &json)
        .with_context(|| format!("Failed to write manifest to '{}'", manifest_path.display()))?;

    println!(
        "{} Manifest written to {}",
        style("✓").green(),
        manifest_path.display()
    );
    Ok(())
}

fn snapshot_skill(config: &AppConfig, name: &str, output: Option<&str>) -> Result<()> {
    let skill_dir = config.skills.root_dir.join(name);
    if !skill_dir.exists() {
        anyhow::bail!("Skill '{}' not found at '{}'", name, skill_dir.display());
    }

    let output_path = output
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("{}.tar.gz", name)));

    let file = std::fs::File::create(&output_path)
        .with_context(|| format!("Failed to create '{}'", output_path.display()))?;

    let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);

    tar.append_dir_all(name, &skill_dir)
        .with_context(|| format!("Failed to archive skill '{}'", name))?;

    let _ = tar.finish().context("Failed to finalize archive")?;

    println!(
        "{} Snapshot written to {}",
        style("✓").green(),
        output_path.display()
    );
    Ok(())
}

fn handle_tap_command(config: &AppConfig, cmd: TapCommand) -> Result<()> {
    let taps_path = config.skills.root_dir.join("taps.json");

    match cmd {
        TapCommand::List => {
            if !taps_path.exists() {
                println!("No taps configured.");
                return Ok(());
            }
            let raw = std::fs::read_to_string(&taps_path).context("Failed to read taps.json")?;

            let taps: HashMap<String, String> =
                serde_json::from_str(&raw).context("Failed to parse taps.json")?;

            if taps.is_empty() {
                println!("No taps configured.");
            } else {
                println!("Configured taps:");
                for (alias, source) in &taps {
                    println!("  {} → {}", style(alias).cyan(), source);
                }
            }
        }
        TapCommand::Add { source, alias } => {
            let alias = alias.unwrap_or_else(|| derive_name_from_url(&source));

            let mut taps: HashMap<String, String> = if taps_path.exists() {
                let raw =
                    std::fs::read_to_string(&taps_path).context("Failed to read taps.json")?;
                serde_json::from_str(&raw).unwrap_or_default()
            } else {
                HashMap::new()
            };

            taps.insert(alias.clone(), source.clone());
            let json = serde_json::to_string_pretty(&taps).context("Failed to serialize taps")?;
            std::fs::write(&taps_path, &json).context("Failed to write taps.json")?;

            println!("{} Tap added: {} → {}", style("✓").green(), alias, source);
        }
        TapCommand::Remove { name } => {
            if !taps_path.exists() {
                anyhow::bail!("No taps configured.");
            }

            let raw = std::fs::read_to_string(&taps_path).context("Failed to read taps.json")?;

            let mut taps: HashMap<String, String> =
                serde_json::from_str(&raw).context("Failed to parse taps.json")?;

            if taps.remove(&name).is_none() {
                anyhow::bail!("Tap '{}' not found.", name);
            }

            let json = serde_json::to_string_pretty(&taps).context("Failed to serialize taps")?;
            std::fs::write(&taps_path, &json).context("Failed to write taps.json")?;

            println!("{} Tap removed: {}", style("✓").green(), name);
        }
    }

    Ok(())
}

fn toggle_skill(config: &AppConfig, name: &str) -> Result<()> {
    let skill_dir = config.skills.root_dir.join(name);

    if !skill_dir.exists() {
        anyhow::bail!("Skill '{}' not found at '{}'", name, skill_dir.display());
    }

    let skill_file = skill_dir.join("SKILL.md");
    let disabled_file = skill_dir.join("SKILL.md.disabled");

    if skill_file.exists() {
        std::fs::rename(&skill_file, &disabled_file)
            .with_context(|| format!("Failed to disable skill '{}'", name))?;
        println!(
            "{} Skill '{}' disabled (SKILL.md → SKILL.md.disabled).",
            style("✓").green(),
            name
        );
    } else if disabled_file.exists() {
        std::fs::rename(&disabled_file, &skill_file)
            .with_context(|| format!("Failed to enable skill '{}'", name))?;
        println!(
            "{} Skill '{}' enabled (SKILL.md.disabled → SKILL.md).",
            style("✓").green(),
            name
        );
    } else {
        anyhow::bail!(
            "Skill '{}' has neither SKILL.md nor SKILL.md.disabled.",
            name
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Reconstruct SKILL.md content from a loaded Skill struct.
fn reconstruct_skill_md(skill: &operant_core::skills::Skill) -> String {
    let mut front = String::from("---\n");
    front.push_str(&format!("name: {}\n", skill.name));
    front.push_str(&format!("description: {}\n", skill.description));
    front.push_str(&format!("version: {}\n", skill.version));

    if !skill.platforms.is_empty() {
        front.push_str(&format!("platforms: [{}]\n", skill.platforms.join(", ")));
    }
    if !skill.prerequisites_env.is_empty() {
        front.push_str(&format!(
            "prerequisites_env: [{}]\n",
            skill.prerequisites_env.join(", ")
        ));
    }
    if !skill.prerequisites_commands.is_empty() {
        front.push_str(&format!(
            "prerequisites_commands: [{}]\n",
            skill.prerequisites_commands.join(", ")
        ));
    }
    front.push_str("---\n");

    if !skill.content.is_empty() {
        front.push_str(&skill.content);
        if !skill.content.ends_with('\n') {
            front.push('\n');
        }
    }

    front
}

/// Derive a skill name from a URL by taking the last path segment.
fn derive_name_from_url(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("skill")
        .trim_end_matches(".md")
        .to_string()
}

/// Return the current platform string (matching skills.rs convention).
fn current_platform() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    }
}
