//! CLI profile management subcommand.
//!
//! Implements `hermes profile list`, `hermes profile show`, `hermes profile create`,
//! `hermes profile use`, `hermes profile delete`, `hermes profile alias`,
//! `hermes profile rename`, `hermes profile export`, and `hermes profile import`.
//!
//! Profiles are stored as individual TOML files in `<hermes_home>/profiles/`.
//! The active profile is tracked via a sentinel file at
//! `<hermes_home>/profiles/active` containing the profile name.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Subcommand;
use hermes_core::config::AppConfig;
use hermes_core::platform::hermes_home;
use serde::{Deserialize, Serialize};

/// Profile subcommand variants.
#[derive(Debug, Clone, Subcommand)]
pub enum ProfileSubcommand {
    /// List all available profiles
    List,
    /// Show profile details (active profile if no name given)
    Show {
        /// Profile name to display; defaults to the active profile
        name: Option<String>,
    },
    /// Create a new profile
    Create {
        /// Profile name
        name: String,
        /// Model identifier (e.g. "gpt-4o", "claude-3-opus-20240229")
        #[arg(long)]
        model: Option<String>,
        /// Base URL for the API endpoint
        #[arg(long)]
        base_url: Option<String>,
    },
    /// Switch to a profile (make it active)
    Use {
        /// Profile name to activate
        name: String,
    },
    /// Delete a profile
    Delete {
        /// Profile name to delete
        name: String,
    },
    /// Alias a profile (copy to a new name)
    Alias {
        /// Existing profile name
        name: String,
        /// New alias name
        alias: String,
    },
    /// Rename a profile
    Rename {
        /// Current profile name
        old_name: String,
        /// New profile name
        new_name: String,
    },
    /// Export a profile to a TOML file
    Export {
        /// Profile name to export
        name: String,
        /// Output file path (defaults to <name>.toml in the current directory)
        output: Option<PathBuf>,
    },
    /// Import a profile from a TOML file
    Import {
        /// Path to the TOML file to import
        path: PathBuf,
        /// Name for the imported profile (defaults to the name field in the file)
        name: Option<String>,
    },
    /// Install a profile from a source path
    Install {
        /// Profile name
        name: String,
        /// Source TOML file path
        source: Option<String>,
    },
    /// Update a profile (or all if no name given)
    Update {
        /// Profile name to update; updates all if omitted
        name: Option<String>,
    },
    /// Show detailed info about a profile
    Info {
        /// Profile name to inspect
        name: String,
    },
}

/// A saved Hermes profile persisted as a TOML file.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Profile {
    name: String,
    model: String,
    base_url: Option<String>,
    api_key_hint: Option<String>,
    created_at: String,
    updated_at: String,
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Directory containing all profile TOML files.
fn profiles_dir() -> PathBuf {
    hermes_home().join("profiles")
}

/// Sentinel file holding the name of the active profile.
fn active_profile_path() -> PathBuf {
    profiles_dir().join("active")
}

/// Path to an individual profile's TOML file.
fn profile_path(name: &str) -> PathBuf {
    profiles_dir().join(format!("{name}.toml"))
}

/// Create the profiles directory if it does not exist.
fn ensure_profiles_dir() -> Result<()> {
    std::fs::create_dir_all(profiles_dir()).context("Failed to create profiles directory")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Active profile sentinel
// ---------------------------------------------------------------------------

/// Read the name of the currently active profile, if any.
fn read_active_profile() -> Result<Option<String>> {
    let path = active_profile_path();
    if path.exists() {
        let name = std::fs::read_to_string(&path)
            .context("Failed to read active profile sentinel")?
            .trim()
            .to_string();
        if name.is_empty() {
            return Ok(None);
        }
        Ok(Some(name))
    } else {
        Ok(None)
    }
}

/// Write the active profile sentinel.
fn write_active_profile(name: &str) -> Result<()> {
    std::fs::write(active_profile_path(), name)
        .context("Failed to write active profile sentinel")?;
    Ok(())
}

/// Remove the active profile sentinel.
fn clear_active_profile() -> Result<()> {
    let path = active_profile_path();
    if path.exists() {
        std::fs::remove_file(&path).context("Failed to remove active profile sentinel")?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Profile I/O
// ---------------------------------------------------------------------------

/// Load a profile from its TOML file.
fn read_profile(name: &str) -> Result<Profile> {
    let path = profile_path(name);
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read profile '{name}'"))?;
    let profile: Profile = toml::from_str(&content)
        .with_context(|| format!("Failed to parse profile '{name}'"))?;
    Ok(profile)
}

/// Write a profile to its TOML file.
fn write_profile(profile: &Profile) -> Result<()> {
    let content = toml::to_string_pretty(profile)
        .with_context(|| format!("Failed to serialize profile '{}'", profile.name))?;
    std::fs::write(profile_path(&profile.name), content)
        .with_context(|| format!("Failed to write profile '{}'", profile.name))?;
    Ok(())
}

/// List all profile names in the profiles directory (sorted).
fn list_profile_names() -> Result<Vec<String>> {
    let dir = profiles_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&dir).context("Failed to read profiles directory")? {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();
        if path.extension().map_or(false, |ext| ext == "toml") {
            if let Some(stem) = path.file_stem() {
                names.push(stem.to_string_lossy().to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Mask an API key for safe display (first 4 + last 4 characters).
fn mask_api_key(key: &str) -> String {
    if key.len() <= 8 {
        return "****".to_string();
    }
    let prefix = &key[..4];
    let suffix = &key[key.len() - 4..];
    format!("{prefix}...{suffix}")
}

/// Build an API key hint from an optional raw key value.
fn build_api_key_hint(key: Option<&String>) -> Option<String> {
    let key = key?;
    if key.is_empty() {
        return None;
    }
    Some(mask_api_key(key))
}

/// Produce an ISO-8601 UTC timestamp string from the current system time.
fn iso_timestamp_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let total_secs = dur.as_secs();
    let z = total_secs / 86_400; // days since epoch
    let rem = total_secs % 86_400;

    let h = rem / 3_600;
    let m = (rem % 3_600) / 60;
    let s = rem % 60;

    // Convert days-since-epoch to year / month / day.
    let (year, month, day) = days_to_date(z as i64);

    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Convert days since 1970-01-01 to a (year, month, day) tuple.
fn days_to_date(mut days: i64) -> (i64, u32, u32) {
    let mut year = 1970i64;
    loop {
        let dim = if is_leap_year(year) { 366 } else { 365 };
        if days < dim {
            break;
        }
        days -= dim;
        year += 1;
    }

    let month_days = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 0u32;
    for (i, &md) in month_days.iter().enumerate() {
        if days < md {
            month = i as u32;
            break;
        }
        days -= md;
    }
    let day = (days + 1) as u32;
    (year, month + 1, day)
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Dispatch a profile subcommand.
pub async fn handle_profile_command(config: &AppConfig, cmd: ProfileSubcommand) -> Result<()> {
    match cmd {
        ProfileSubcommand::List => cmd_list(),
        ProfileSubcommand::Show { name } => cmd_show(name),
        ProfileSubcommand::Create {
            name,
            model,
            base_url,
        } => cmd_create(config, name, model, base_url),
        ProfileSubcommand::Use { name } => cmd_use(name),
        ProfileSubcommand::Delete { name } => cmd_delete(name),
        ProfileSubcommand::Alias { name, alias } => cmd_alias(name, alias),
        ProfileSubcommand::Rename {
            old_name,
            new_name,
        } => cmd_rename(old_name, new_name),
        ProfileSubcommand::Export { name, output } => cmd_export(name, output),
        ProfileSubcommand::Import { path, name } => cmd_import(path, name),
        ProfileSubcommand::Install { name, source } => cmd_install(config, name, source),
        ProfileSubcommand::Update { name } => cmd_update(name),
        ProfileSubcommand::Info { name } => cmd_info(name),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// List all available profiles in a table.
fn cmd_list() -> Result<()> {
    ensure_profiles_dir()?;
    let active = read_active_profile()?;
    let names = list_profile_names()?;

    if names.is_empty() {
        println!("No profiles found. Create one with `hermes profile create <name>`.");
        return Ok(());
    }

    println!(
        "{:<4} {:<28} {:<24} {:<24}",
        "#", "Name", "Model", "Base URL"
    );
    println!("{}", "-".repeat(84));
    for (i, name) in names.iter().enumerate() {
        let marker = if active.as_deref() == Some(name.as_str()) {
            "*"
        } else {
            " "
        };
        match read_profile(name) {
            Ok(profile) => {
                let base = profile.base_url.as_deref().unwrap_or("(default)");
                println!(
                    "{} {:<3} {:<28} {:<24} {:<24}",
                    marker,
                    i + 1,
                    profile.name,
                    profile.model,
                    base,
                );
            }
            Err(e) => {
                println!(
                    "{} {:<3} {:<28} {:<24} {:<24}",
                    marker,
                    i + 1,
                    name,
                    "(error)",
                    e.to_string().chars().take(24).collect::<String>(),
                );
            }
        }
    }

    if active.is_some() {
        println!();
        println!("* = active profile");
    }

    Ok(())
}

/// Show detailed information about a profile.
fn cmd_show(name: Option<String>) -> Result<()> {
    ensure_profiles_dir()?;

    let profile_name = match name {
        Some(n) => n,
        None => read_active_profile()?
            .ok_or_else(|| anyhow::anyhow!("No active profile and no profile name given"))?,
    };

    let profile = read_profile(&profile_name)?;
    let active = read_active_profile()?;
    let is_active = active.as_deref() == Some(&profile_name);

    println!("── Profile: {} ──────────────────────────", profile_name);
    if is_active {
        println!("  Status:        (active)");
    }
    println!("  Name:          {}", profile.name);
    println!("  Model:         {}", profile.model);
    println!(
        "  Base URL:      {}",
        profile.base_url.as_deref().unwrap_or("(default)")
    );
    println!(
        "  API key:       {}",
        profile.api_key_hint.as_deref().unwrap_or("(not set)")
    );
    println!("  Created:       {}", profile.created_at);
    println!("  Updated:       {}", profile.updated_at);
    println!("───────────────────────────────────────────");

    Ok(())
}

/// Create a new profile using current configuration defaults.
fn cmd_create(
    config: &AppConfig,
    name: String,
    model: Option<String>,
    base_url: Option<String>,
) -> Result<()> {
    ensure_profiles_dir()?;

    if profile_path(&name).exists() {
        anyhow::bail!("Profile '{name}' already exists");
    }

    let now = iso_timestamp_now();

    let resolved_model = model.unwrap_or_else(|| config.agent.model.clone());

    let resolved_base_url = base_url.or_else(|| {
        let url = &config.client.base_url;
        if url.is_empty() || url == "https://api.openai.com/v1" {
            None
        } else {
            Some(url.clone())
        }
    });

    let api_key_hint = build_api_key_hint(config.client.api_key.as_ref());

    let profile = Profile {
        name: name.clone(),
        model: resolved_model,
        base_url: resolved_base_url,
        api_key_hint,
        created_at: now.clone(),
        updated_at: now,
    };

    write_profile(&profile)?;
    println!("Created profile '{name}'");
    Ok(())
}

/// Switch the active profile.
fn cmd_use(name: String) -> Result<()> {
    ensure_profiles_dir()?;

    if !profile_path(&name).exists() {
        anyhow::bail!("Profile '{name}' does not exist");
    }

    write_active_profile(&name)?;
    println!("Switched to profile '{name}'");
    Ok(())
}

/// Delete a profile and clear the active sentinel if it was active.
fn cmd_delete(name: String) -> Result<()> {
    ensure_profiles_dir()?;

    if !profile_path(&name).exists() {
        anyhow::bail!("Profile '{name}' does not exist");
    }

    std::fs::remove_file(profile_path(&name))
        .with_context(|| format!("Failed to delete profile '{name}'"))?;

    if let Some(active) = read_active_profile()? {
        if active == name {
            clear_active_profile()?;
        }
    }

    println!("Deleted profile '{name}'");
    Ok(())
}

/// Alias a profile by copying its TOML file.
fn cmd_alias(name: String, alias: String) -> Result<()> {
    ensure_profiles_dir()?;

    if !profile_path(&name).exists() {
        anyhow::bail!("Profile '{name}' does not exist");
    }
    if profile_path(&alias).exists() {
        anyhow::bail!("Profile '{alias}' already exists");
    }

    let mut profile = read_profile(&name)?;
    profile.name = alias.clone();
    profile.updated_at = iso_timestamp_now();
    write_profile(&profile)?;

    println!("Aliased profile '{name}' as '{alias}'");
    Ok(())
}

/// Rename a profile by moving its TOML file.
fn cmd_rename(old_name: String, new_name: String) -> Result<()> {
    ensure_profiles_dir()?;

    if !profile_path(&old_name).exists() {
        anyhow::bail!("Profile '{old_name}' does not exist");
    }
    if profile_path(&new_name).exists() {
        anyhow::bail!("Profile '{new_name}' already exists");
    }

    std::fs::rename(profile_path(&old_name), profile_path(&new_name))
        .with_context(|| format!("Failed to rename profile '{old_name}' to '{new_name}'"))?;

    // Update the active sentinel if the renamed profile was active.
    if let Some(active) = read_active_profile()? {
        if active == old_name {
            write_active_profile(&new_name)?;
        }
    }

    println!("Renamed profile '{old_name}' to '{new_name}'");
    Ok(())
}

/// Export a profile to a TOML file at a given (or default) path.
fn cmd_export(name: String, output: Option<PathBuf>) -> Result<()> {
    ensure_profiles_dir()?;

    let profile = read_profile(&name)?;
    let content = toml::to_string_pretty(&profile)
        .with_context(|| format!("Failed to serialize profile '{name}'"))?;

    let output_path = output.unwrap_or_else(|| PathBuf::from(format!("{name}.toml")));
    std::fs::write(&output_path, &content)
        .with_context(|| format!("Failed to write to '{}'", output_path.display()))?;

    println!(
        "Exported profile '{name}' to {}",
        output_path.display()
    );
    Ok(())
}

/// Import a profile from a TOML file.
fn cmd_import(path: PathBuf, rename: Option<String>) -> Result<()> {
    ensure_profiles_dir()?;

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read '{}'", path.display()))?;
    let mut profile: Profile = toml::from_str(&content)
        .with_context(|| format!("Failed to parse profile from '{}'", path.display()))?;

    if let Some(new_name) = rename {
        profile.name = new_name;
    }

    if profile_path(&profile.name).exists() {
        anyhow::bail!("Profile '{}' already exists", profile.name);
    }

    profile.updated_at = iso_timestamp_now();
    write_profile(&profile)?;

    println!(
        "Imported profile '{}' from {}",
        profile.name,
        path.display()
    );
    Ok(())
}

/// Install a profile from a source TOML file path.
fn cmd_install(config: &AppConfig, name: String, source: Option<String>) -> Result<()> {
    ensure_profiles_dir()?;

    if profile_path(&name).exists() {
        anyhow::bail!("Profile '{name}' already exists");
    }

    let profile = match source {
        Some(path) => {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read source '{}'", path))?;
            let mut p: Profile = toml::from_str(&content)
                .with_context(|| format!("Failed to parse source '{}'", path))?;
            p.name = name.clone();
            p.updated_at = iso_timestamp_now();
            p
        }
        None => {
            let now = iso_timestamp_now();
            Profile {
                name: name.clone(),
                model: config.agent.model.clone(),
                base_url: {
                    let url = &config.client.base_url;
                    if url.is_empty() || url == "https://api.openai.com/v1" {
                        None
                    } else {
                        Some(url.clone())
                    }
                },
                api_key_hint: build_api_key_hint(config.client.api_key.as_ref()),
                created_at: now.clone(),
                updated_at: now,
            }
        }
    };

    write_profile(&profile)?;
    println!("Installed profile '{name}'");
    Ok(())
}

/// Update a profile's timestamp (or all profiles if no name given).
fn cmd_update(name: Option<String>) -> Result<()> {
    ensure_profiles_dir()?;

    let names = match name {
        Some(n) => {
            if !profile_path(&n).exists() {
                anyhow::bail!("Profile '{n}' does not exist");
            }
            vec![n]
        }
        None => list_profile_names()?,
    };

    if names.is_empty() {
        println!("No profiles to update.");
        return Ok(());
    }

    for n in &names {
        let mut profile = read_profile(n)
            .with_context(|| format!("Failed to read profile '{n}'"))?;
        profile.updated_at = iso_timestamp_now();
        write_profile(&profile)?;
        println!("Updated profile '{n}'");
    }

    Ok(())
}

/// Show detailed info about a profile.
fn cmd_info(name: String) -> Result<()> {
    ensure_profiles_dir()?;

    if !profile_path(&name).exists() {
        anyhow::bail!("Profile '{name}' does not exist");
    }

    let profile = read_profile(&name)?;
    let active = read_active_profile()?;
    let is_active = active.as_deref() == Some(&name);

    println!("── Profile: {} ──────────────────────────", name);
    if is_active {
        println!("  Status:        (active)");
    }
    println!("  Name:          {}", profile.name);
    println!("  Model:         {}", profile.model);
    println!(
        "  Base URL:      {}",
        profile.base_url.as_deref().unwrap_or("(default)")
    );
    println!(
        "  API key:       {}",
        profile.api_key_hint.as_deref().unwrap_or("(not set)")
    );
    println!("  Created:       {}", profile.created_at);
    println!("  Updated:       {}", profile.updated_at);
    println!("───────────────────────────────────────────");

    Ok(())
}
