//! CLI subcommand for managing skills.
//!
//! Provides skill discovery and inspection:
//! - `hermes skills list` — list installed skills
//! - `hermes skills search <query>` — search available skills
//! - `hermes skills inspect <id>` — show details of a specific skill

use anyhow::{Context, Result};
use clap::Subcommand;
use hermes_core::config::AppConfig;
use hermes_core::skills::SkillManager;

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
}

pub async fn handle_skills_command(config: &AppConfig, cmd: SkillsSubcommand) -> Result<()> {
    match cmd {
        SkillsSubcommand::List => list_skills(config),
        SkillsSubcommand::Search { query } => search_skills(config, &query),
        SkillsSubcommand::Inspect { id } => inspect_skill(config, &id),
    }
}

fn list_skills(config: &AppConfig) -> Result<()> {
    let skills_dir = &config.skills.root_dir;

    if !skills_dir.exists() {
        println!("No skills installed.");
        return Ok(());
    }

    let mut manager = SkillManager::new(skills_dir.clone());
    manager
        .load_all()
        .with_context(|| format!("Failed to load skills from '{}'", skills_dir.display()))?;

    let skills = manager.list();

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
    let all = manager.load_all().with_context(|| {
        format!("Failed to load skills from '{}'", skills_dir.display())
    })?;

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
