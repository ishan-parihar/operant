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
//! 3. Calls [`Composition::resolve`](crate::composition::Composition::resolve)
//!    and returns the final [`Architecture`].
//!
//! When the base file or any patch is missing, the result is
//! `Architecture::default()` (empty rows) — boot is permitted with no
//! providers, mirroring the dark-merge default.

use std::path::{Path, PathBuf};

use crate::composition::{Architecture, Patch};
use crate::error::HarnessError;
use crate::pool::{LEGACY_CONFIG_NAME, ORG_CONFIG_NAME, OrgManifest};

/// Default patch directory under the user's operant state dir.
pub const DEFAULT_PATCH_DIR: &str = ".operant/patches";

/// Default organism root, relative to `$HOME`.
pub const DEFAULT_ORGANISM_REL: &str = ".hermes/organism";

/// The strata an organism root is divided into, as declared by
/// `organism/_org.yaml` under `structure.strata` and mirrored by the
/// organism's own `STRATA` constant in `cortex/meta-governance/lib/core.py`.
pub const ORGANISM_STRATA: &[&str] = &["cortex", "swarm", "foundations", "ventures"];

/// A pool discovered on disk: where its manifest is and what it declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredPool {
    /// The pool directory (the parent of the manifest).
    pub dir: PathBuf,
    /// The manifest file, `_org.yaml` or `_pool.yaml`.
    pub manifest: PathBuf,
    /// `pool.name` from the manifest.
    pub name: String,
    /// The stratum the pool sits in, when it is under a known one.
    pub stratum: Option<String>,
    /// `pool.parent` — `None` for a top-level pool.
    pub parent: Option<String>,
}

/// Resolve a directory to the pool that owns it (AD-050 "integration point"
/// resolution: a path anywhere inside a pool belongs to that pool).
///
/// Walks upward from `path` looking for a directory that carries a pool
/// manifest, using the organism's own precedence: `_org.yaml` first,
/// `_pool.yaml` as the legacy fallback. This is what makes a deep
/// `sub-systems/` path route to its owning pool instead of failing.
///
/// Symlinks are resolved before walking, because AD-060 pooled organs are
/// symlinks into a foundation: routing through the symlink should land on
/// the foundation that actually owns the genome, not on the graduate that
/// borrowed it.
pub fn route(path: &Path) -> Result<DiscoveredPool, HarnessError> {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut cursor: Option<&Path> = Some(resolved.as_path());
    while let Some(dir) = cursor {
        if let Some((manifest, name)) = read_pool_identity(dir) {
            return Ok(DiscoveredPool {
                dir: dir.to_path_buf(),
                manifest,
                name,
                stratum: stratum_of(dir),
                parent: read_parent(dir),
            });
        }
        cursor = dir.parent();
    }
    Err(HarnessError::CompositionError(format!(
        "no pool manifest found at or above {} — a pool directory carries \
         `{ORG_CONFIG_NAME}` (canonical) or `{LEGACY_CONFIG_NAME}` (legacy)",
        resolved.display()
    )))
}

/// Enumerate every pool under an organism root.
///
/// Scans the declared strata for top-level pools, then recurses into
/// `sub-systems/` for native sub-system organs. **Symlinks are skipped**,
/// matching the organism's own `get_all_pool_dirs` (a symlinked foundation
/// is an integration point, not a second canonical pool) and
/// `emit_architecture_registry` (cycle protection). Real directories that
/// a symlink also points at are still discovered through the stratum that
/// owns them.
pub fn list(root: &Path) -> Result<Vec<DiscoveredPool>, HarnessError> {
    if !root.is_dir() {
        return Err(HarnessError::CompositionError(format!(
            "organism root {} is not a directory",
            root.display()
        )));
    }
    let mut out: Vec<DiscoveredPool> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();

    for stratum in ORGANISM_STRATA {
        let s_dir = root.join(stratum);
        if !s_dir.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&s_dir) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && !p.is_symlink() && !is_hidden_name(p))
            .collect();
        dirs.sort();
        for dir in dirs {
            collect_recursive(&dir, &mut out, &mut seen)?;
        }
    }
    Ok(out)
}

/// The default organism root: `$HOME/.hermes/organism`, or `None` when
/// `$HOME` is not set.
pub fn default_organism_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let mut p = PathBuf::from(home);
    p.push(DEFAULT_ORGANISM_REL);
    Some(p)
}

/// List pools under the default organism root.
pub fn list_default() -> Result<Vec<DiscoveredPool>, HarnessError> {
    let root = default_organism_root().ok_or_else(|| {
        HarnessError::CompositionError("cannot determine $HOME for the organism root".to_string())
    })?;
    list(&root)
}

/// The manifest in `dir`, honouring the organism's `_org.yaml` > `_pool.yaml`
/// precedence.
pub fn manifest_path(dir: &Path) -> Option<PathBuf> {
    let org = dir.join(ORG_CONFIG_NAME);
    if org.is_file() {
        return Some(org);
    }
    let legacy = dir.join(LEGACY_CONFIG_NAME);
    if legacy.is_file() {
        return Some(legacy);
    }
    None
}

/// Read `pool.name` from a pool directory's manifest, if it has one.
pub fn read_pool_identity(dir: &Path) -> Option<(PathBuf, String)> {
    let manifest = manifest_path(dir)?;
    let raw = std::fs::read_to_string(&manifest).ok()?;
    let parsed: OrgManifest = serde_yaml::from_str(&raw).ok()?;
    let name = parsed.pool.name.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some((manifest, name))
}

fn read_parent(dir: &Path) -> Option<String> {
    let manifest = manifest_path(dir)?;
    let raw = std::fs::read_to_string(&manifest).ok()?;
    let parsed: OrgManifest = serde_yaml::from_str(&raw).ok()?;
    parsed.pool.parent
}

fn stratum_of(dir: &Path) -> Option<String> {
    for stratum in ORGANISM_STRATA {
        if dir.components().any(|c| c.as_os_str() == *stratum) {
            return Some((*stratum).to_string());
        }
    }
    None
}

fn is_hidden_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with('.') || n.starts_with('_'))
        .unwrap_or(true)
}

fn collect_recursive(
    dir: &Path,
    out: &mut Vec<DiscoveredPool>,
    seen: &mut Vec<PathBuf>,
) -> Result<(), HarnessError> {
    // Canonicalize so a directory reachable by two paths is listed once.
    let key = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    if seen.contains(&key) {
        return Ok(());
    }
    seen.push(key);

    if let Some((manifest, name)) = read_pool_identity(dir) {
        out.push(DiscoveredPool {
            dir: dir.to_path_buf(),
            manifest,
            name,
            stratum: stratum_of(dir),
            parent: read_parent(dir),
        });
    }
    let sub_dir = dir.join("sub-systems");
    if sub_dir.is_dir() {
        let Ok(entries) = std::fs::read_dir(&sub_dir) else {
            return Ok(());
        };
        let mut subs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            // AD-060: a symlink inside sub-systems/ is an integration point
            // (AD-050), not a canonical pool location. Skip it, or the walk
            // would fan out through a foundation and duplicate every pool
            // under every graduate that pools it.
            .filter(|p| p.is_dir() && !p.is_symlink() && !is_hidden_name(p))
            .collect();
        subs.sort();
        for sub in subs {
            collect_recursive(&sub, out, seen)?;
        }
    }
    Ok(())
}

/// Resolve the boot architecture: load the base + ordered patches.
pub fn resolve_boot_architecture(
    base_path: &Path,
    patch_dir: Option<&Path>,
) -> Result<Architecture, HarnessError> {
    let raw = std::fs::read_to_string(base_path).map_err(|e| {
        HarnessError::CompositionError(format!(
            "read architecture.toml {}: {e}",
            base_path.display()
        ))
    })?;
    let mut arch = Architecture::from_toml(&raw).map_err(|e| {
        HarnessError::CompositionError(format!(
            "parse architecture.toml {}: {e}",
            base_path.display()
        ))
    })?;

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
    let entries = std::fs::read_dir(dir).map_err(|e| {
        HarnessError::CompositionError(format!("read patch dir {}: {e}", dir.display()))
    })?;
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("toml") && p.is_file())
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let raw = std::fs::read_to_string(&path).map_err(|e| {
            HarnessError::CompositionError(format!("read patch {}: {e}", path.display()))
        })?;
        let patch: Patch = toml::from_str(&raw).map_err(|e| {
            HarnessError::CompositionError(format!("parse patch {}: {e}", path.display()))
        })?;
        out.push(patch);
    }
    Ok(out)
}

/// Resolve the standard patch directory: `$HOME/.operant/patches` on
/// unix, `$USERPROFILE\.operant\patches` on Windows. Returns `None` if
/// the home dir cannot be determined.
pub fn default_patch_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
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
