//! Profile management for multiple isolated Hermes instances.
//!
//! Each profile is a fully independent HERMES_HOME directory with its own
//! config, memory, sessions, skills, and logs. Profiles live under
//! `~/.hermes/profiles/<name>/` by default.
//!
//! The "default" profile is `~/.hermes` itself — backward compatible,
//! zero migration needed.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::platform;

/// Profile identifier regex: lowercase alphanumeric, hyphens, underscores.
const PROFILE_ID_RE: &str = r"^[a-z0-9][a-z0-9_-]{0,63}$";

/// Reserved profile names that cannot be used.
const RESERVED_NAMES: &[&str] = &["hermes", "default", "test", "tmp", "root", "sudo"];

thread_local! {
    static HERMES_HOME_OVERRIDE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Set a thread-local override for HERMES_HOME.
///
/// Returns a token that can be used to restore the previous value.
pub fn set_hermes_home_override(path: impl Into<PathBuf>) -> HermesHomeToken {
    let previous = HERMES_HOME_OVERRIDE.with(|cell| cell.replace(Some(path.into())));
    HermesHomeToken(previous)
}

/// Reset the HERMES_HOME override using a token from `set_hermes_home_override`.
pub fn reset_hermes_home_override(token: HermesHomeToken) {
    HERMES_HOME_OVERRIDE.with(|cell| cell.replace(token.0));
}

/// Token for restoring HERMES_HOME override.
pub struct HermesHomeToken(Option<PathBuf>);

/// Get the current HERMES_HOME with profile awareness.
///
/// Resolution order:
/// 1. Thread-local override (from `set_hermes_home_override`)
/// 2. `HERMES_HOME` environment variable
/// 3. Default: `~/.hermes`
pub fn get_hermes_home() -> PathBuf {
    // 1. Check thread-local override
    let override_path = HERMES_HOME_OVERRIDE.with(|cell| cell.borrow().clone());
    if let Some(path) = override_path {
        return path;
    }

    // 2. Check environment variable
    if let Ok(val) = std::env::var("HERMES_HOME") {
        if !val.is_empty() {
            return PathBuf::from(val);
        }
    }

    // 3. Default
    platform::hermes_home()
}

/// Get the default hermes root (before profile resolution).
///
/// In standard deployments this is `~/.hermes`.
/// In Docker/custom deployments where HERMES_HOME points outside `~/.hermes`,
/// returns HERMES_HOME directly.
pub fn get_default_hermes_root() -> PathBuf {
    let native_home = platform::hermes_home();
    let env_home = std::env::var("HERMES_HOME").unwrap_or_default();
    if env_home.is_empty() {
        return native_home;
    }

    let env_path = PathBuf::from(&env_home);
    // Check if HERMES_HOME is under ~/.hermes (normal or profile mode)
    if let (Ok(env_resolved), Ok(native_resolved)) =
        (env_path.canonicalize(), native_home.canonicalize())
    {
        if env_resolved.starts_with(&native_resolved) {
            return native_home;
        }
    }

    // Docker / custom deployment: check if this is a profile path
    if env_path
        .parent()
        .map(|p| p.ends_with("profiles"))
        .unwrap_or(false)
    {
        return env_path.parent().unwrap().parent().unwrap().to_path_buf();
    }

    // Not a profile path — HERMES_HOME itself is the root
    env_path
}

/// Get the profiles root directory.
pub fn get_profiles_root() -> PathBuf {
    get_default_hermes_root().join("profiles")
}

/// Get the active profile file path.
fn get_active_profile_path() -> PathBuf {
    get_default_hermes_root().join("active_profile")
}

/// Get the currently active profile name.
///
/// Returns "default" if no active_profile file exists.
pub fn get_active_profile() -> String {
    let path = get_active_profile_path();
    if let Ok(content) = fs::read_to_string(&path) {
        let name = content.trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }
    "default".to_string()
}

/// Set the active profile (sticky default).
pub fn set_active_profile(name: &str) -> Result<()> {
    let path = get_active_profile_path();
    fs::write(&path, format!("{}\n", name)).map_err(|e| {
        Error::Config(format!(
            "Failed to write active_profile to '{}': {}",
            path.display(),
            e
        ))
    })?;
    Ok(())
}

/// Normalize a profile name (lowercase, trimmed).
pub fn normalize_profile_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(Error::Config("Profile name cannot be empty".to_string()));
    }
    if trimmed.eq_ignore_ascii_case("default") {
        return Ok("default".to_string());
    }
    Ok(trimmed.to_lowercase())
}

/// Validate a profile name.
pub fn validate_profile_name(name: &str) -> Result<()> {
    if name == "default" {
        return Ok(());
    }

    let re = regex::Regex::new(PROFILE_ID_RE).unwrap();
    if !re.is_match(name) {
        return Err(Error::Config(format!(
            "Invalid profile name '{}'. Must match [a-z0-9][a-z0-9_-]{{0,63}}",
            name
        )));
    }

    if RESERVED_NAMES.contains(&name) {
        return Err(Error::Config(format!(
            "Profile name '{}' is reserved — it collides with the Hermes installation itself.",
            name
        )));
    }

    Ok(())
}

/// Get the directory for a profile.
pub fn get_profile_dir(name: &str) -> Result<PathBuf> {
    let canon = normalize_profile_name(name)?;
    if canon == "default" {
        Ok(get_default_hermes_root())
    } else {
        Ok(get_profiles_root().join(&canon))
    }
}

/// Check if a profile exists.
pub fn profile_exists(name: &str) -> bool {
    let canon = match normalize_profile_name(name) {
        Ok(c) => c,
        Err(_) => return false,
    };
    if canon == "default" {
        return true;
    }
    let dir = get_profiles_root().join(&canon);
    dir.is_dir()
}

/// Information about a profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileInfo {
    /// Profile name.
    pub name: String,
    /// Path to the profile directory.
    pub path: PathBuf,
    /// Whether this is the default profile.
    pub is_default: bool,
    /// Model configured in this profile's config.
    pub model: Option<String>,
    /// Number of skills installed.
    pub skill_count: usize,
}

/// Count installed skills in a profile directory.
fn count_skills(profile_dir: &Path) -> usize {
    let skills_dir = profile_dir.join("skills");
    if !skills_dir.is_dir() {
        return 0;
    }
    let mut count = 0;
    if let Ok(entries) = fs::read_dir(&skills_dir) {
        for entry in entries.flatten() {
            if entry.path().join("SKILL.md").exists() {
                count += 1;
            }
        }
    }
    count
}

/// Read model from a profile's config file.
fn read_config_model(profile_dir: &Path) -> Option<String> {
    let config_path = profile_dir.join("hermes.toml");
    if !config_path.exists() {
        return None;
    }
    let content = fs::read_to_string(&config_path).ok()?;
    let config: toml::Value = toml::from_str(&content).ok()?;
    config
        .get("agent")?
        .get("model")?
        .as_str()
        .map(|s| s.to_string())
}

/// List all profiles.
pub fn list_profiles() -> Result<Vec<ProfileInfo>> {
    let mut profiles = Vec::new();

    // Default profile
    let default_home = get_default_hermes_root();
    if default_home.is_dir() {
        profiles.push(ProfileInfo {
            name: "default".to_string(),
            path: default_home.clone(),
            is_default: true,
            model: read_config_model(&default_home),
            skill_count: count_skills(&default_home),
        });
    }

    // Named profiles
    let profiles_root = get_profiles_root();
    if profiles_root.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(&profiles_root)
            .map_err(|e| {
                Error::Config(format!(
                    "Failed to read profiles directory '{}': {}",
                    profiles_root.display(),
                    e
                ))
            })?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .collect();

        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "default" {
                continue;
            }
            // Validate name format
            if validate_profile_name(&name).is_err() {
                continue;
            }

            let dir = entry.path();
            profiles.push(ProfileInfo {
                name: name.clone(),
                path: dir.clone(),
                is_default: false,
                model: read_config_model(&dir),
                skill_count: count_skills(&dir),
            });
        }
    }

    Ok(profiles)
}

/// Create a new profile.
pub fn create_profile(name: &str, clone_from: Option<&str>) -> Result<PathBuf> {
    let canon = normalize_profile_name(name)?;
    validate_profile_name(&canon)?;

    if canon == "default" {
        return Err(Error::Config(
            "Cannot create a profile named 'default' — it is the built-in profile (~/.hermes)."
                .to_string(),
        ));
    }

    let profile_dir = get_profiles_root().join(&canon);
    if profile_dir.exists() {
        return Err(Error::Config(format!(
            "Profile '{}' already exists at {}",
            canon,
            profile_dir.display()
        )));
    }

    // Resolve clone source
    let source_dir = if let Some(source_name) = clone_from {
        let source_canon = normalize_profile_name(source_name)?;
        let source_dir = get_profile_dir(&source_canon)?;
        if !source_dir.is_dir() {
            return Err(Error::Config(format!(
                "Source profile '{}' does not exist at {}",
                source_canon,
                source_dir.display()
            )));
        }
        Some(source_dir)
    } else {
        None
    };

    // Create profile directory structure
    fs::create_dir_all(&profile_dir).map_err(|e| {
        Error::Config(format!(
            "Failed to create profile directory '{}': {}",
            profile_dir.display(),
            e
        ))
    })?;

    // Create standard subdirectories
    for subdir in &["config", "memories", "sessions", "skills", "logs"] {
        fs::create_dir_all(profile_dir.join(subdir)).map_err(|e| {
            Error::Config(format!(
                "Failed to create subdirectory '{}/{}': {}",
                canon, subdir, e
            ))
        })?;
    }

    // Clone config files from source
    if let Some(source) = source_dir {
        for filename in &["hermes.toml", ".env", "SOUL.md"] {
            let src = source.join(filename);
            if src.exists() {
                let dst = profile_dir.join(filename);
                fs::copy(&src, &dst).map_err(|e| {
                    Error::Config(format!(
                        "Failed to copy '{}' to '{}': {}",
                        src.display(),
                        dst.display(),
                        e
                    ))
                })?;
            }
        }

        // Clone skills
        let source_skills = source.join("skills");
        if source_skills.is_dir() {
            copy_dir_recursive(&source_skills, &profile_dir.join("skills"))?;
        }
    }

    Ok(profile_dir)
}

/// Delete a profile.
pub fn delete_profile(name: &str) -> Result<PathBuf> {
    let canon = normalize_profile_name(name)?;
    validate_profile_name(&canon)?;

    if canon == "default" {
        return Err(Error::Config(
            "Cannot delete the default profile (~/.hermes).".to_string(),
        ));
    }

    let profile_dir = get_profiles_root().join(&canon);
    if !profile_dir.is_dir() {
        return Err(Error::Config(format!(
            "Profile '{}' does not exist.",
            canon
        )));
    }

    // Clear active_profile if it points to this profile
    if get_active_profile() == canon {
        set_active_profile("default")?;
    }

    // Remove profile directory
    fs::remove_dir_all(&profile_dir).map_err(|e| {
        Error::Config(format!(
            "Failed to remove profile directory '{}': {}",
            profile_dir.display(),
            e
        ))
    })?;

    Ok(profile_dir)
}

/// Switch to a profile (set as sticky default).
pub fn use_profile(name: &str) -> Result<PathBuf> {
    let canon = normalize_profile_name(name)?;
    validate_profile_name(&canon)?;

    if !profile_exists(&canon) {
        return Err(Error::Config(format!(
            "Profile '{}' does not exist.",
            canon
        )));
    }

    set_active_profile(&canon)?;
    get_profile_dir(&canon)
}

/// Clone a profile.
pub fn clone_profile(source_name: &str, target_name: &str) -> Result<PathBuf> {
    create_profile(target_name, Some(source_name))
}

/// Copy a directory recursively.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).map_err(|e| {
        Error::Config(format!(
            "Failed to create directory '{}': {}",
            dst.display(),
            e
        ))
    })?;

    for entry in fs::read_dir(src).map_err(|e| {
        Error::Config(format!(
            "Failed to read directory '{}': {}",
            src.display(),
            e
        ))
    })? {
        let entry = entry.map_err(|e| {
            Error::Config(format!(
                "Failed to read entry in '{}': {}",
                src.display(),
                e
            ))
        })?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path).map_err(|e| {
                Error::Config(format!(
                    "Failed to copy '{}' to '{}': {}",
                    src_path.display(),
                    dst_path.display(),
                    e
                ))
            })?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "hermes_profile_test_{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn test_normalize_profile_name() {
        assert_eq!(normalize_profile_name("default").unwrap(), "default");
        assert_eq!(normalize_profile_name("Default").unwrap(), "default");
        assert_eq!(normalize_profile_name("  coder  ").unwrap(), "coder");
        assert_eq!(normalize_profile_name("CODER").unwrap(), "coder");
        assert!(normalize_profile_name("").is_err());
        assert!(normalize_profile_name("  ").is_err());
    }

    #[test]
    fn test_validate_profile_name() {
        assert!(validate_profile_name("default").is_ok());
        assert!(validate_profile_name("coder").is_ok());
        assert!(validate_profile_name("my-profile").is_ok());
        assert!(validate_profile_name("my_profile").is_ok());
        assert!(validate_profile_name("123profile").is_ok());
        assert!(validate_profile_name("hermes").is_err());
        assert!(validate_profile_name("test").is_err());
        assert!(validate_profile_name("-invalid").is_err());
        assert!(validate_profile_name("INVALID").is_err());
        assert!(validate_profile_name("has spaces").is_err());
    }

    #[test]
    fn test_profile_exists() {
        // Default always exists
        assert!(profile_exists("default"));
        // Non-existent profile
        assert!(!profile_exists("nonexistent_profile_12345"));
    }

    #[test]
    fn test_list_profiles_includes_default() {
        let profiles = list_profiles().unwrap();
        assert!(profiles.iter().any(|p| p.name == "default" && p.is_default));
    }

    #[test]
    fn test_create_and_delete_profile() {
        let _guard = env_lock().lock().unwrap();
        let dir = temp_dir("create_delete");
        let original = env::var("HERMES_HOME").ok();

        env::set_var("HERMES_HOME", &dir);

        // Create profile
        let profile_dir = create_profile("testprofile", None).unwrap();
        assert!(profile_dir.exists());
        assert!(profile_exists("testprofile"));

        // Verify structure
        assert!(profile_dir.join("skills").exists());
        assert!(profile_dir.join("memories").exists());
        assert!(profile_dir.join("sessions").exists());

        // Delete profile
        let deleted = delete_profile("testprofile").unwrap();
        assert_eq!(deleted, profile_dir);
        assert!(!profile_dir.exists());
        assert!(!profile_exists("testprofile"));

        // Cleanup
        let _ = fs::remove_dir_all(dir);
        match original {
            Some(val) => env::set_var("HERMES_HOME", val),
            None => env::remove_var("HERMES_HOME"),
        }
    }

    #[test]
    fn test_clone_profile() {
        let _guard = env_lock().lock().unwrap();
        let dir = temp_dir("clone");
        let original = env::var("HERMES_HOME").ok();

        env::set_var("HERMES_HOME", &dir);

        // Create source profile
        let source_dir = create_profile("source", None).unwrap();
        let config_content = "[agent]\nmodel = \"gpt-4\"\n";
        fs::write(source_dir.join("hermes.toml"), config_content).unwrap();

        // Clone to target
        let target_dir = clone_profile("source", "target").unwrap();
        assert!(target_dir.exists());
        assert!(profile_exists("target"));

        // Verify config was copied
        let target_config = fs::read_to_string(target_dir.join("hermes.toml")).unwrap();
        assert_eq!(target_config, config_content);

        // Cleanup
        let _ = fs::remove_dir_all(dir);
        match original {
            Some(val) => env::set_var("HERMES_HOME", val),
            None => env::remove_var("HERMES_HOME"),
        }
    }

    #[test]
    fn test_use_profile() {
        let _guard = env_lock().lock().unwrap();
        let dir = temp_dir("use_profile");
        let original = env::var("HERMES_HOME").ok();

        env::set_var("HERMES_HOME", &dir);

        // Create profile
        create_profile("switchto", None).unwrap();

        // Switch to it
        use_profile("switchto").unwrap();
        assert_eq!(get_active_profile(), "switchto");

        // Switch back to default
        use_profile("default").unwrap();
        assert_eq!(get_active_profile(), "default");

        // Cleanup
        let _ = fs::remove_dir_all(dir);
        match original {
            Some(val) => env::set_var("HERMES_HOME", val),
            None => env::remove_var("HERMES_HOME"),
        }
    }

    #[test]
    fn test_get_hermes_home_override() {
        let _guard = env_lock().lock().unwrap();
        let original = env::var("HERMES_HOME").ok();

        let test_path = PathBuf::from("/tmp/test_override");
        let token = set_hermes_home_override(&test_path);
        assert_eq!(get_hermes_home(), test_path);

        reset_hermes_home_override(token);
        // Should fall back to env var or default
        env::set_var("HERMES_HOME", "/tmp/test_env");
        assert_eq!(get_hermes_home(), PathBuf::from("/tmp/test_env"));

        match original {
            Some(val) => env::set_var("HERMES_HOME", val),
            None => env::remove_var("HERMES_HOME"),
        }
    }

    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }
}
