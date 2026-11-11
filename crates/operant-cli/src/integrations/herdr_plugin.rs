//! The Herdr plugin: install, inspect, and repair it.
//!
//! Herdr discovers a plugin from a `herdr-plugin.toml` and registers it with
//! `herdr plugin link <dir>`. Its docs are explicit that `link` is the
//! authoring path ("When you are authoring a local plugin, link the working
//! directory instead"), and that linking registers the plugin while no Herdr
//! server need be running — so this installer works from a plain shell.
//!
//! WHAT THIS PLUGIN IS FOR, AND WHAT IT IS NOT
//!
//! Operant reports its own state; it does not need a plugin to be seen by
//! Herdr (see `operant_core::herdr`). The plugin is the human-facing surface
//! around that: a pane entrypoint to start a session, a setup action, and a
//! doctor action. Installing it is what makes Operant *discoverable* from
//! inside Herdr — the difference between "herdr can see my operant sessions"
//! and "herdr has an Operant entry in its plugin list".

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The plugin id, used for the install directory and for `herdr plugin link`.
pub const PLUGIN_ID: &str = "operant.integration";

/// The manifest, compiled in so the installed copy always matches the binary.
const MANIFEST: &str = include_str!("../../integrations/herdr/herdr-plugin.toml");
const LAUNCH_SH: &str = include_str!("../../integrations/herdr/scripts/launch.sh");
const SETUP_SH: &str = include_str!("../../integrations/herdr/scripts/setup.sh");
const DOCTOR_SH: &str = include_str!("../../integrations/herdr/scripts/doctor.sh");

/// One file the plugin is made of.
struct Asset {
    /// Path relative to the plugin root.
    relative: &'static str,
    contents: &'static str,
    /// Scripts are made executable; the manifest is not.
    executable: bool,
}

const ASSETS: &[Asset] = &[
    Asset {
        relative: "herdr-plugin.toml",
        contents: MANIFEST,
        executable: false,
    },
    Asset {
        relative: "scripts/launch.sh",
        contents: LAUNCH_SH,
        executable: true,
    },
    Asset {
        relative: "scripts/setup.sh",
        contents: SETUP_SH,
        executable: true,
    },
    Asset {
        relative: "scripts/doctor.sh",
        contents: DOCTOR_SH,
        executable: true,
    },
];

/// Where the plugin lives once installed.
///
/// Kept under the Operant home rather than inside Herdr's own plugin tree:
/// Herdr's `link` command is designed for a plugin the user owns elsewhere,
/// and everything Herdr writes for the plugin (its config and its state)
/// already lives under `~/.config/herdr/plugins/`.
pub fn plugin_root() -> PathBuf {
    operant_core::platform::operant_home()
        .join("integrations")
        .join("herdr")
        .join(PLUGIN_ID)
}

/// The herdr binary to talk to.
///
/// Inside a Herdr pane the pane names its own binary; outside one, the PATH
/// `herdr` is used.
pub fn herdr_bin() -> String {
    std::env::var("HERDR_BIN_PATH")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "herdr".to_string())
}

/// Whether a herdr binary answering `--version` is reachable.
pub fn herdr_available() -> bool {
    std::process::Command::new(herdr_bin())
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// The install state of the plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginStatus {
    /// Whether the plugin directory holds every asset at the shipped content.
    pub files_current: bool,
    /// Assets that are missing or differ from the shipped copy.
    pub stale: Vec<String>,
    /// Whether `herdr plugin list` reports the plugin.
    pub linked: bool,
}

/// Inspect the installed plugin against the shipped assets.
pub fn status() -> PluginStatus {
    status_at(&plugin_root())
}

/// Inspect a plugin root against the shipped assets.
///
/// Split from [`status`] so tests can point it at a temporary directory.
pub fn status_at(root: &Path) -> PluginStatus {
    let mut stale = Vec::new();
    for asset in ASSETS {
        let path = root.join(asset.relative);
        let current = std::fs::read_to_string(&path)
            .map(|existing| existing == asset.contents)
            .unwrap_or(false);
        if !current {
            stale.push(asset.relative.to_string());
        }
    }
    PluginStatus {
        files_current: stale.is_empty(),
        stale,
        linked: is_linked(),
    }
}

/// Whether Herdr reports the plugin as installed.
fn is_linked() -> bool {
    if !herdr_available() {
        return false;
    }
    std::process::Command::new(herdr_bin())
        .args(["plugin", "list"])
        .output()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout).contains(PLUGIN_ID)
                || String::from_utf8_lossy(&output.stderr).contains(PLUGIN_ID)
        })
        .unwrap_or(false)
}

/// Write the plugin and register it with Herdr.
///
/// Idempotent: unchanged files are left alone (so a re-run does not churn
/// mtimes), and an already-linked plugin is not linked again.
pub fn install() -> Result<Vec<String>> {
    let root = plugin_root();
    write_assets(&root)?;
    let mut done = vec![format!("wrote the Herdr plugin to {}", root.display())];

    if !herdr_available() {
        done.push(
            "herdr is not on PATH — the plugin is written but not registered; \
             install Herdr, then run this again"
                .to_string(),
        );
        return Ok(done);
    }

    if is_linked() {
        done.push("herdr already lists the Operant plugin".to_string());
        return Ok(done);
    }

    let output = std::process::Command::new(herdr_bin())
        .arg("plugin")
        .arg("link")
        .arg(&root)
        .output()
        .context("running `herdr plugin link`")?;
    if output.status.success() {
        done.push("registered the plugin with `herdr plugin link`".to_string());
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "`herdr plugin link {}` failed: {}",
            root.display(),
            stderr.trim()
        );
    }
    Ok(done)
}

/// Write every asset under `root`, skipping the ones already current.
fn write_assets(root: &Path) -> Result<()> {
    for asset in ASSETS {
        let path = root.join(asset.relative);
        let current = std::fs::read_to_string(&path)
            .map(|existing| existing == asset.contents)
            .unwrap_or(false);
        if current {
            // Content is right, but a lost exec bit would still break the
            // scripts, so the mode is checked separately.
            if asset.executable {
                make_executable(&path)?;
            }
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&path, asset.contents)
            .with_context(|| format!("writing {}", path.display()))?;
        if asset.executable {
            make_executable(&path)?;
        }
    }
    Ok(())
}

/// Mark a file executable for its owner.
///
/// Unix mode bits only; on a platform without them the scripts would not run
/// anyway, and Herdr reports that rather than this failing silently.
pub fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)
            .with_context(|| format!("stat {}", path.display()))?
            .permissions();
        // 0o755: the owner's bit is what matters, the rest keeps the file
        // readable so the plugin tree can be inspected.
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)
            .with_context(|| format!("chmod 755 {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_root_reports_every_asset_stale() {
        let dir = tempfile::tempdir().expect("tempdir");
        let status = status_at(dir.path());
        assert!(!status.files_current);
        assert_eq!(status.stale.len(), ASSETS.len());
    }

    #[test]
    fn writing_the_assets_makes_the_root_current() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_assets(dir.path()).expect("write assets");
        let status = status_at(dir.path());
        assert!(status.files_current, "stale: {:?}", status.stale);
        assert!(dir.path().join("herdr-plugin.toml").exists());
        assert!(dir.path().join("scripts/launch.sh").exists());
    }

    #[test]
    fn a_modified_asset_is_reported_stale_and_restored() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_assets(dir.path()).expect("write assets");
        let launch = dir.path().join("scripts/launch.sh");
        std::fs::write(&launch, "#!/bin/sh\necho tampered\n").expect("tamper");

        let status = status_at(dir.path());
        assert_eq!(status.stale, vec!["scripts/launch.sh".to_string()]);

        write_assets(dir.path()).expect("rewrite");
        assert!(status_at(dir.path()).files_current);
        assert_eq!(
            std::fs::read_to_string(&launch).expect("read"),
            LAUNCH_SH,
            "the shipped copy is restored verbatim"
        );
    }

    #[cfg(unix)]
    #[test]
    fn scripts_are_executable_and_the_manifest_is_not() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        write_assets(dir.path()).expect("write assets");

        let mode = |rel: &str| {
            std::fs::metadata(dir.path().join(rel))
                .expect("stat")
                .permissions()
                .mode()
                & 0o111
        };
        assert_ne!(mode("scripts/launch.sh"), 0, "launch.sh must be executable");
        assert_ne!(mode("scripts/setup.sh"), 0, "setup.sh must be executable");
        assert_eq!(
            mode("herdr-plugin.toml"),
            0,
            "the manifest is data, not code"
        );
    }

    #[test]
    fn the_manifest_names_the_id_herdr_registers() {
        assert!(
            MANIFEST.contains(&format!("id = \"{PLUGIN_ID}\"")),
            "the manifest id must match the id link/status look for"
        );
    }
}
