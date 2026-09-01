//! G7 — `architecture.toml` boot discovery.
//!
//! Operators maintain a base `architecture.toml` (committed to the
//! repo) and an ordered set of `architecture.patch.toml` overlays
//! under `~/.operant/patches/` (or a configured directory). This
//! module:
//!
//! 1. Resolves the base path (`config.harness.architecture_toml`).
//! 2. Discovers patches in `~/.operant/patches/*.toml` in sorted order
//!    (so operators get a deterministic boot).
//! 3. Calls [`Composition::resolve`] and returns the final
//!    [`Architecture`].
//!
//! When the base file or any patch is missing, the result is
//! `Architecture::default()` (empty rows) — boot is permitted with no
//! providers, mirroring the dark-merge default.

use std::path::{Path, PathBuf};

use crate::composition::{Architecture, Patch};
use crate::error::HarnessError;

/// Default patch directory under the user's operant state dir.
pub const DEFAULT_PATCH_DIR: &str = ".operant/patches";

/// Resolve the boot architecture: load the base + ordered patches.
pub fn resolve_boot_architecture(
    base_path: &Path,
    patch_dir: Option<&Path>,
) -> Result<Architecture, HarnessError> {
    let raw = std::fs::read_to_string(base_path).map_err(|e| HarnessError::CompositionError(
        format!("read architecture.toml {}: {e}", base_path.display()),
    ))?;
    let mut arch = Architecture::from_toml(&raw).map_err(|e| HarnessError::CompositionError(
        format!("parse architecture.toml {}: {e}", base_path.display()),
    ))?;

    let patches = collect_patches(patch_dir)?;
    if !patches.is_empty() {
        tracing::info!(
            count = patches.len(),
            dir = %patch_dir.map(|p| p.display().to_string()).unwrap_or_default(),
            "applying harness architecture patches"
        );
        arch = crate::composition::Composition::resolve(arch, &patches)?;
    }
    Ok(arch)
}

/// Load every `*.toml` file in `dir` (or empty when `None`) and parse
/// each as a [`Patch`]. Returned in sorted (lexicographic) order so
/// the operator gets a deterministic boot.
pub fn collect_patches(dir: Option<&Path>) -> Result<Vec<Patch>, HarnessError> {
    let Some(dir) = dir else {
        return Ok(Vec::new());
    };
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(dir).map_err(|e| HarnessError::CompositionError(
        format!("read patch dir {}: {e}", dir.display()),
    ))?;
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("toml")
                && p.is_file()
        })
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let raw = std::fs::read_to_string(&path).map_err(|e| HarnessError::CompositionError(
            format!("read patch {}: {e}", path.display()),
        ))?;
        let patch: Patch = toml::from_str(&raw).map_err(|e| HarnessError::CompositionError(
            format!("parse patch {}: {e}", path.display()),
        ))?;
        out.push(patch);
    }
    Ok(out)
}

/// Resolve the standard patch directory: `$HOME/.operant/patches` on
/// unix, `$USERPROFILE\.operant\patches` on Windows. Returns `None` if
/// the home dir cannot be determined.
pub fn default_patch_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))?;
    let mut p = PathBuf::from(home);
    p.push(DEFAULT_PATCH_DIR);
    Some(p)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn collect_patches_returns_empty_when_dir_missing() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let patches = collect_patches(Some(&missing)).expect("missing dir is OK");
        assert!(patches.is_empty());
    }

    #[test]
    fn collect_patches_sorts_lexicographically() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("02-second.toml"),
            r#"
[[disable]]
id = "x"
"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("01-first.toml"),
            r#"
[[disable]]
id = "y"
"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("README.md"), "ignored").unwrap();
        let patches = collect_patches(Some(dir.path())).expect("collect");
        assert_eq!(patches.len(), 2);
    }

    #[test]
    fn resolve_boot_architecture_composes_patches() {
        let dir = tempdir().unwrap();
        let base = dir.path().join("architecture.toml");
        std::fs::write(
            &base,
            r#"
[[rows]]
id = "alpha"
source = "native"
disabled = false
kind = "native"

[[rows]]
id = "beta"
source = "native"
disabled = false
kind = "native"
"#,
        )
        .unwrap();
        let patch_dir = dir.path().join("patches");
        std::fs::create_dir(&patch_dir).unwrap();
        std::fs::write(
            patch_dir.join("01-disable-alpha.toml"),
            r#"
[[disable]]
id = "alpha"
"#,
        )
        .unwrap();
        let arch = resolve_boot_architecture(&base, Some(&patch_dir)).expect("resolve");
        let active: Vec<&str> = arch.active().map(|r| r.id.as_str()).collect();
        assert_eq!(active, vec!["beta"]);
    }
}
