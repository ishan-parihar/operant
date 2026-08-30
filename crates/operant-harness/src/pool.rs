//! Phase 6 — hermes pool import compiler (plan 016).
//!
//! Reads `~/.hermes/systems/<pool>/_pool.yaml` and compiles it to a
//! vector of [`crate::composition::ArchitectureRow`]s the kernel's
//! [`crate::composition::Builder`] can mount.
//!
//! ## Mapping
//!
//! `services_offered`     ⇒ claims (each `name` becomes a `Claim`)
//! `services_consumed`    ⇒ `requires` on the family provider
//! `pooled_sub_systems`   ⇒ `kind = "pool.bundle"` config_rows; each
//!                          row's config is `{ "path": "...", "read_only": true }`
//!
//! Read-only is the default for v1: pool adapters cannot mutate the
//! pool directly. The compiler hard-fails when a row claims a
//! `services_offered` name that doesn't start with a known read-only
//! verb (see [`READ_ONLY_VERBS`]).

use std::path::Path;

use serde::Deserialize;

use crate::composition::ArchitectureRow;
use crate::error::HarnessError;

/// The verbs we accept for v1. Any `services_offered` whose name starts
/// with one of these is read-only; anything else is rejected.
pub const READ_ONLY_VERBS: &[&str] = &[
    "query.", "fetch.", "list.", "search.", "get.", "read.", "lookup.",
];

/// Raw shape of `~/.hermes/systems/<pool>/_pool.yaml` (subset we care
/// about; extra fields are ignored by `serde(default)`).
#[derive(Debug, Clone, Deserialize)]
pub struct PoolManifest {
    /// Pool display name.
    #[serde(default)]
    pub name: String,
    /// Services this pool offers to other pools / the loop. Each name
    /// becomes a Claim the kernel can depend on.
    #[serde(default)]
    pub services_offered: Vec<PoolService>,
    /// Services this pool needs from other pools. Each name becomes a
    /// `requires` claim on the family provider.
    #[serde(default)]
    pub services_consumed: Vec<PoolService>,
    /// Bundled sub-systems. Each becomes a `kind=pool.bundle`
    /// config_row that the host's boot pass routes to a read-only
    /// adapter.
    #[serde(default)]
    pub pooled_sub_systems: Vec<PoolSubSystem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PoolService {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PoolSubSystem {
    pub name: String,
    pub path: String,
}

/// Compiler output. Each entry is a row the kernel can mount.
#[derive(Debug, Clone)]
pub struct CompiledPool {
    pub name: String,
    pub family_row: ArchitectureRow,
    pub bundle_rows: Vec<ArchitectureRow>,
    pub claims: Vec<String>,
}

/// Compile a pool manifest. Returns the family provider row (claims
/// + requires) and one row per `pooled_sub_system`.
pub fn compile(manifest: &PoolManifest) -> Result<CompiledPool, HarnessError> {
    if manifest.name.is_empty() {
        return Err(HarnessError::CompositionError(
            "pool manifest has empty `name`".to_string(),
        ));
    }
    // Validate offered services are read-only verbs
    for svc in &manifest.services_offered {
        let n = &svc.name;
        if !READ_ONLY_VERBS.iter().any(|v| n.starts_with(v)) {
            return Err(HarnessError::CompositionError(format!(
                "pool `{}` offers non-read-only service `{}`; v1 is read-only",
                manifest.name, n
            )));
        }
    }
    // Build the family row
    let mut claims: Vec<String> = Vec::new();
    for svc in &manifest.services_offered {
        claims.push(svc.name.clone());
    }
    let requires: Vec<String> = manifest
        .services_consumed
        .iter()
        .map(|s| s.name.clone())
        .collect();
    let family_id = format!("pool.{}", manifest.name);
    let family_row = ArchitectureRow {
        id: family_id.clone(),
        source: "pool".to_string(),
        disabled: false,
        config: serde_json::json!({
            "name": manifest.name,
            "claims": claims,
            "requires": requires,
        }),
        kind: Some("pool.family".to_string()),
    };
    // Build the bundle rows
    let mut bundle_rows = Vec::new();
    for sub in &manifest.pooled_sub_systems {
        bundle_rows.push(ArchitectureRow {
            id: format!("pool.{}.{}", manifest.name, sub.name),
            source: "pool".to_string(),
            disabled: false,
            config: serde_json::json!({
                "path": sub.path,
                "read_only": true,
            }),
            kind: Some("pool.bundle".to_string()),
        });
    }
    Ok(CompiledPool {
        name: manifest.name.clone(),
        family_row,
        bundle_rows,
        claims,
    })
}

/// Load a `_pool.yaml` from disk and compile it.
pub fn load_and_compile(path: &Path) -> Result<CompiledPool, HarnessError> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        HarnessError::CompositionError(format!("read pool manifest {}: {e}", path.display()))
    })?;
    let manifest: PoolManifest = serde_yaml::from_str(&raw)
        .map_err(|e| HarnessError::CompositionError(format!("yaml parse pool manifest: {e}")))?;
    compile(&manifest)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn manifest_yaml() -> &'static str {
        r#"
name: relationship-intel
services_offered:
  - name: query.contacts
    description: Find contacts by name/email
  - name: search.history
    description: Search communication history
services_consumed:
  - name: auth.identity
    description: Identity provider
pooled_sub_systems:
  - name: contacts
    path: ~/.hermes/systems/relationship-intel/contacts
  - name: history
    path: ~/.hermes/systems/relationship-intel/history
"#
    }

    #[test]
    fn compiles_read_only_pool() {
        let m: PoolManifest = serde_yaml::from_str(manifest_yaml()).unwrap();
        let compiled = compile(&m).unwrap();
        assert_eq!(compiled.name, "relationship-intel");
        assert_eq!(compiled.claims, vec!["query.contacts", "search.history"]);
        assert_eq!(compiled.family_row.id, "pool.relationship-intel");
        assert_eq!(compiled.family_row.source, "pool");
        assert_eq!(compiled.family_row.kind.as_deref(), Some("pool.family"));
        assert_eq!(compiled.bundle_rows.len(), 2);
        assert_eq!(compiled.bundle_rows[0].kind.as_deref(), Some("pool.bundle"));
        assert!(
            compiled.bundle_rows[0]
                .config
                .get("read_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        );
    }

    #[test]
    fn rejects_write_verbs() {
        let yaml = r#"
name: bad-pool
services_offered:
  - name: delete.contacts
    description: Delete a contact (forbidden in v1)
"#;
        let m: PoolManifest = serde_yaml::from_str(yaml).unwrap();
        let err = compile(&m).unwrap_err();
        assert!(err.to_string().contains("non-read-only"));
        assert!(err.to_string().contains("delete.contacts"));
    }

    #[test]
    fn rejects_empty_name() {
        let yaml = "name: \"\"";
        let m: PoolManifest = serde_yaml::from_str(yaml).unwrap();
        let err = compile(&m).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn load_and_compile_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("_pool.yaml");
        std::fs::write(&path, manifest_yaml()).unwrap();
        let compiled = load_and_compile(&path).unwrap();
        assert_eq!(compiled.name, "relationship-intel");
    }
}
