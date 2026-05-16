//! Plugin installation from Git repositories.
//! Ported from hermes-agent/hermes_cli/plugins_cmd.py.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// Install a plugin from a git URL or owner/repo shorthand.
/// Returns the plugin name (derived from directory name).
pub async fn install_plugin(identifier: &str, plugins_dir: &Path, force: bool) -> Result<String> {
    // Resolve identifier to a git URL
    let git_url = resolve_git_url(identifier);
    let plugin_name = derive_plugin_name(identifier);

    let target = plugins_dir.join(&plugin_name);

    if target.exists() {
        if force {
            std::fs::remove_dir_all(&target).with_context(|| {
                format!("Failed to remove existing plugin at {}", target.display())
            })?;
        } else {
            anyhow::bail!(
                "Plugin '{}' is already installed at {}. Use --force to reinstall.",
                plugin_name,
                target.display()
            );
        }
    }

    // Ensure parent directory exists
    std::fs::create_dir_all(plugins_dir).with_context(|| {
        format!(
            "Failed to create plugins directory at {}",
            plugins_dir.display()
        )
    })?;

    // Clone the repository
    let status = Command::new("git")
        .args(["clone", "--depth", "1", &git_url, &target.to_string_lossy()])
        .status()
        .context("Failed to execute git clone. Is git installed?")?;

    if !status.success() {
        anyhow::bail!("git clone failed for URL: {}", git_url);
    }

    // Validate plugin manifest
    validate_plugin(&target)?;

    Ok(plugin_name)
}

/// Resolve an identifier to a git URL.
/// Supports: full URLs, owner/repo shorthand -> https://github.com/owner/repo
fn resolve_git_url(identifier: &str) -> String {
    if identifier.starts_with("http://")
        || identifier.starts_with("https://")
        || identifier.starts_with("git@")
        || identifier.starts_with("file://")
    {
        identifier.to_string()
    } else if let Some((owner, repo)) = identifier.split_once('/') {
        // Owner/repo shorthand -> GitHub
        format!("https://github.com/{}/{}", owner, repo)
    } else {
        // Assume it's a full name, try GitHub
        format!("https://github.com/{}/{}", identifier, identifier)
    }
}

/// Derive a plugin name from the identifier (last path segment, no .git).
fn derive_plugin_name(identifier: &str) -> String {
    let name = if identifier.ends_with(".git") {
        &identifier[..identifier.len() - 4]
    } else {
        identifier
    };
    let name = name.trim_end_matches('/');
    name.rsplit_once('/')
        .map(|(_, last)| last.to_string())
        .unwrap_or_else(|| name.to_string())
}

/// Validate that a plugin directory has a valid plugin.yaml manifest.
fn validate_plugin(plugin_dir: &Path) -> Result<()> {
    let manifest_path = plugin_dir.join("plugin.yaml");
    if !manifest_path.exists() {
        // Fallback: check for __init__.py
        let init_path = plugin_dir.join("__init__.py");
        if !init_path.exists() {
            anyhow::bail!(
                "No plugin.yaml or __init__.py found in {}. Is this a valid Hermes plugin?",
                plugin_dir.display()
            );
        }
        return Ok(());
    }

    let content = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;

    // Basic YAML check: must have a name field
    if !content.contains("name:") && !content.contains("name :") {
        anyhow::bail!("plugin.yaml is missing a 'name' field.");
    }

    Ok(())
}

/// Remove an installed plugin.
pub fn remove_plugin(name: &str, plugins_dir: &Path) -> Result<()> {
    let target = plugins_dir.join(name);
    if !target.exists() {
        anyhow::bail!("Plugin '{}' is not installed.", name);
    }
    std::fs::remove_dir_all(&target)
        .with_context(|| format!("Failed to remove plugin '{}'", name))?;
    Ok(())
}

/// List installed plugins.
pub fn list_plugins(plugins_dir: &Path) -> Result<Vec<String>> {
    if !plugins_dir.exists() {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = std::fs::read_dir(plugins_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    Ok(names)
}

/// Check that git is available on the system.
pub fn check_git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_resolve_git_url_https() {
        assert_eq!(
            resolve_git_url("https://github.com/owner/repo.git"),
            "https://github.com/owner/repo.git"
        );
    }

    #[test]
    fn test_resolve_git_url_git_ssh() {
        assert_eq!(
            resolve_git_url("git@github.com:owner/repo.git"),
            "git@github.com:owner/repo.git"
        );
    }

    #[test]
    fn test_resolve_git_url_owner_repo_shorthand() {
        assert_eq!(
            resolve_git_url("owner/repo"),
            "https://github.com/owner/repo"
        );
    }

    #[test]
    fn test_resolve_git_url_single_name() {
        assert_eq!(
            resolve_git_url("my-plugin"),
            "https://github.com/my-plugin/my-plugin"
        );
    }

    #[test]
    fn test_derive_plugin_name_from_url() {
        assert_eq!(
            derive_plugin_name("https://github.com/owner/my-plugin.git"),
            "my-plugin"
        );
    }

    #[test]
    fn test_derive_plugin_name_from_owner_repo() {
        assert_eq!(derive_plugin_name("owner/my-plugin"), "my-plugin");
    }

    #[test]
    fn test_derive_plugin_name_single() {
        assert_eq!(derive_plugin_name("my-plugin"), "my-plugin");
    }

    #[test]
    fn test_derive_plugin_name_with_trailing_slash() {
        assert_eq!(derive_plugin_name("owner/repo/"), "repo");
    }

    #[test]
    fn test_validate_plugin_with_plugin_yaml() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("plugin.yaml"),
            "name: test-plugin\nversion: 1.0",
        )
        .unwrap();
        assert!(validate_plugin(dir.path()).is_ok());
    }

    #[test]
    fn test_validate_plugin_with_init_py() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("__init__.py"), "# plugin").unwrap();
        assert!(validate_plugin(dir.path()).is_ok());
    }

    #[test]
    fn test_validate_plugin_missing_manifest() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = validate_plugin(dir.path());
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No plugin.yaml or __init__.py"));
    }

    #[test]
    fn test_validate_plugin_yaml_missing_name() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("plugin.yaml"),
            "version: 1.0\ndescription: foo",
        )
        .unwrap();
        let result = validate_plugin(dir.path());
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("missing a 'name' field"));
    }

    #[test]
    fn test_list_plugins_empty_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let plugins = list_plugins(dir.path()).unwrap();
        assert!(plugins.is_empty());
    }

    #[test]
    fn test_list_plugins_with_entries() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::create_dir(dir.path().join("plugin-a")).unwrap();
        fs::create_dir(dir.path().join("plugin-b")).unwrap();
        let plugins = list_plugins(dir.path()).unwrap();
        assert_eq!(plugins, vec!["plugin-a", "plugin-b"]);
    }

    #[test]
    fn test_remove_plugin() {
        let dir = tempfile::TempDir::new().unwrap();
        let plugin_dir = dir.path().join("test-plugin");
        fs::create_dir(&plugin_dir).unwrap();
        assert!(plugin_dir.exists());
        remove_plugin("test-plugin", dir.path()).unwrap();
        assert!(!plugin_dir.exists());
    }

    #[test]
    fn test_remove_nonexistent_plugin() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = remove_plugin("nonexistent", dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_check_git_available() {
        // This should be true in any dev environment with git installed
        assert!(check_git_available());
    }
}
