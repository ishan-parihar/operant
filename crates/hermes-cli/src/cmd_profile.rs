use anyhow::Result;
use clap::Subcommand;
use hermes_core::config::AppConfig;
use hermes_core::profile::{
    clone_profile, create_profile, delete_profile, get_active_profile, get_profile_dir,
    list_profiles, normalize_profile_name, use_profile,
};

#[derive(Debug, Clone, Subcommand)]
pub enum ProfileSubcommand {
    List,
    Show {
        name: Option<String>,
    },
    Create {
        name: String,
        #[arg(long)]
        clone: Option<String>,
    },
    Use {
        name: String,
    },
    Delete {
        name: String,
    },
    Clone {
        source: String,
        target: String,
    },
}

pub async fn handle_profile_command(_config: &AppConfig, cmd: ProfileSubcommand) -> Result<()> {
    match cmd {
        ProfileSubcommand::List => cmd_list(),
        ProfileSubcommand::Show { name } => cmd_show(name),
        ProfileSubcommand::Create { name, clone } => cmd_create(name, clone),
        ProfileSubcommand::Use { name } => cmd_use(name),
        ProfileSubcommand::Delete { name } => cmd_delete(name),
        ProfileSubcommand::Clone { source, target } => cmd_clone(source, target),
    }
}

fn cmd_list() -> Result<()> {
    let profiles = list_profiles().map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let active = get_active_profile();

    if profiles.is_empty() {
        println!("No profiles found.");
        return Ok(());
    }

    println!("{:<4} {:<24} {:<24} {:<8}", "#", "Name", "Model", "Skills");
    println!("{}", "-".repeat(64));

    for (i, profile) in profiles.iter().enumerate() {
        let marker = if profile.name == active { "*" } else { " " };
        let model = profile.model.as_deref().unwrap_or("(default)");
        println!(
            "{} {:<3} {:<24} {:<24} {:<8}",
            marker,
            i + 1,
            profile.name,
            model,
            profile.skill_count,
        );
    }

    if !active.is_empty() {
        println!();
        println!("* = active profile");
    }

    Ok(())
}

fn cmd_show(name: Option<String>) -> Result<()> {
    let profile_name = name.unwrap_or_else(get_active_profile);
    let canon =
        normalize_profile_name(&profile_name).map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let dir = get_profile_dir(&canon).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    if !dir.is_dir() {
        anyhow::bail!("Profile '{}' does not exist", canon);
    }

    let active = get_active_profile();
    let is_active = active == canon;

    println!("── Profile: {} ──────────────────────────", canon);
    if is_active {
        println!("  Status:        (active)");
    }
    println!("  Path:          {}", dir.display());
    println!("───────────────────────────────────────────");

    Ok(())
}

fn cmd_create(name: String, clone_from: Option<String>) -> Result<()> {
    let result =
        create_profile(&name, clone_from.as_deref()).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!("Created profile '{}' at {}", name, result.display());
    Ok(())
}

fn cmd_use(name: String) -> Result<()> {
    let dir = use_profile(&name).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!("Switched to profile '{}' ({})", name, dir.display());
    Ok(())
}

fn cmd_delete(name: String) -> Result<()> {
    let dir = delete_profile(&name).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!("Deleted profile '{}' ({})", name, dir.display());
    Ok(())
}

fn cmd_clone(source: String, target: String) -> Result<()> {
    let dir = clone_profile(&source, &target).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!(
        "Cloned profile '{}' to '{}' ({})",
        source,
        target,
        dir.display()
    );
    Ok(())
}
