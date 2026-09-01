//! Phase 3 — Composition layer: parse `architecture.toml` rows + apply
//! `architecture.patch.toml` overlays into a resolved provider list.
//!
//! Row format (architecture.toml):
//! ```toml
//! [provider.core]
//! source = "native"           # native | wasm | config_row | pool
//! disabled = false
//!
//! [provider.toolset_extra]
//! source = "config_row"
//! kind = "prompt.section"     # family hint
//! config = { path = "/etc/prompt.md" }
//! ```
//!
//! Patch format (architecture.patch.toml):
//! ```toml
//! [[disable]]
//! id = "old_thing"
//!
//! [[replace]]
//! id = "core"
//! source = "wasm"
//! config = { path = "/opt/core.wasm" }
//!
//! [[insert]]
//! id = "ext_watcher"
//! source = "config_row"
//! kind = "prompt.section"
//! ```
//!
//! Patches are applied in file order; the resulting row set is what
//! `Builder` walks to construct `Vec<Arc<dyn Provider>>` for the kernel.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::HarnessError;

/// One row in the resolved architecture.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchitectureRow {
    /// Stable provider id; must be unique within one composition.
    pub id: String,
    /// Where the provider came from. Pool rows compile to one or more child
    /// rows at boot (Phase 6); for now `Pool` is opaque and not used by the
    /// generic builder.
    pub source: String,
    /// Disabled rows are skipped at boot. Patch-inserts never produce
    /// disabled rows; the patch form `[[disable]]` is the only mutator.
    #[serde(default)]
    pub disabled: bool,
    /// Optional free-form configuration block, passed through to the
    /// provider's `ActivateCx` as JSON.
    #[serde(default)]
    pub config: serde_json::Value,
    /// Optional kind hint (e.g. "prompt.section", "toolset.disable") for
    /// config-row providers. Native/wasm rows leave this None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

impl ArchitectureRow {
    /// Validate the row's required fields. Used by the builder before
    /// dispatching the row to a provider constructor.
    pub fn validate(&self) -> Result<(), HarnessError> {
        if self.id.is_empty() {
            return Err(HarnessError::CompositionError(
                "architecture row has empty id".to_string(),
            ));
        }
        if self.source.is_empty() {
            return Err(HarnessError::CompositionError(format!(
                "architecture row `{}` has empty source",
                self.id
            )));
        }
        if self.source == "config_row" && self.kind.is_none() {
            return Err(HarnessError::CompositionError(format!(
                "architecture row `{}` is config_row but has no `kind`",
                self.id
            )));
        }
        Ok(())
    }
}

/// Resolved set of rows. Holds the original list + a name index for fast
/// patch lookup. Constructed via [`Composition::resolve`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Architecture {
    /// Top-level array of rows. The TOML shape is `[[row]]` (a table-array
    /// at the top level). The struct is also accepted via the more compact
    /// `[provider.<id>]` form by [`Architecture::from_provider_tables`].
    #[serde(default)]
    pub rows: Vec<ArchitectureRow>,
}

impl Architecture {
    /// Parse a `architecture.toml`-style document whose top level is a
    /// sequence of inline tables, each with `id/source/disabled/config/kind`.
    /// The `[provider.<id>]` shape is NOT supported here — call
    /// [`Self::from_provider_tables`] for that.
    pub fn from_toml(s: &str) -> Result<Self, HarnessError> {
        toml::from_str::<Architecture>(s)
            .map_err(|e| HarnessError::CompositionError(format!("toml parse: {e}")))
    }

    /// Render the architecture as a deterministic TOML string suitable for
    /// `operant architecture dump` golden tests.
    pub fn to_toml(&self) -> Result<String, HarnessError> {
        toml::to_string_pretty(self)
            .map_err(|e| HarnessError::CompositionError(format!("toml emit: {e}")))
    }

    /// Validate every row (id/source non-empty; config_row has kind).
    pub fn validate(&self) -> Result<(), HarnessError> {
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for row in &self.rows {
            row.validate()?;
            if !seen.insert(row.id.as_str()) {
                return Err(HarnessError::CompositionError(format!(
                    "duplicate row id `{}`",
                    row.id
                )));
            }
        }
        Ok(())
    }
}

impl Architecture {
    /// Empty architecture — no providers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of rows (including disabled).
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when no rows are present.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Active-only view: skips `disabled` rows.
    pub fn active(&self) -> impl Iterator<Item = &ArchitectureRow> {
        self.rows.iter().filter(|r| !r.disabled)
    }

    /// Find a row by id (active or disabled).
    pub fn find(&self, id: &str) -> Option<&ArchitectureRow> {
        self.rows.iter().find(|r| r.id == id)
    }
}

/// A patch to apply to an [`Architecture`]. See module docs for the TOML
/// shape. Patches are simple to reason about: three operations only.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    /// Disable rows by id. Missing ids are a hard error (we don't silently
    /// succeed on typos).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disable: Vec<PatchTarget>,
    /// Replace a row by id. Source/config/kind are overwritten; the
    /// `disabled` flag is preserved (replacing does not auto-enable).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replace: Vec<ArchitectureRow>,
    /// Insert a new row. If a row with the same id already exists, the
    /// patch fails (use `replace` for mutation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub insert: Vec<ArchitectureRow>,
}

/// A patch target is just an id (used by `[[disable]]`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatchTarget {
    pub id: String,
}

/// Resolver: applies an ordered list of patches to a base architecture.
pub struct Composition;

impl Composition {
    /// Apply patches in order. The first failure aborts and returns
    /// `(error, prior_intact_architecture)` — but because we operate on
    /// owned data, "prior intact" means the caller's original is still
    /// theirs; we never mutate in place.
    pub fn resolve(base: Architecture, patches: &[Patch]) -> Result<Architecture, HarnessError> {
        let mut arch = base;
        for (i, patch) in patches.iter().enumerate() {
            Self::apply(&mut arch, patch)
                .map_err(|e| HarnessError::CompositionError(format!("patch {i}: {e}")))?;
        }
        Ok(arch)
    }

    fn apply(arch: &mut Architecture, patch: &Patch) -> Result<(), HarnessError> {
        // disable
        for tgt in &patch.disable {
            let row = arch
                .rows
                .iter_mut()
                .find(|r| r.id == tgt.id)
                .ok_or_else(|| {
                    HarnessError::CompositionError(format!(
                        "disable target `{}` not found in architecture",
                        tgt.id
                    ))
                })?;
            row.disabled = true;
        }
        // replace
        for new_row in &patch.replace {
            new_row.validate()?;
            let pos = arch
                .rows
                .iter()
                .position(|r| r.id == new_row.id)
                .ok_or_else(|| {
                    HarnessError::CompositionError(format!(
                        "replace target `{}` not found in architecture",
                        new_row.id
                    ))
                })?;
            let mut merged = new_row.clone();
            // preserve the existing `disabled` flag — replacing does not
            // auto-enable a previously-disabled row.
            merged.disabled = arch.rows[pos].disabled;
            arch.rows[pos] = merged;
        }
        // insert
        for new_row in &patch.insert {
            new_row.validate()?;
            if arch.rows.iter().any(|r| r.id == new_row.id) {
                return Err(HarnessError::CompositionError(format!(
                    "insert target `{}` already exists (use replace to mutate)",
                    new_row.id
                )));
            }
            arch.rows.push(new_row.clone());
        }
        // final pass: every row must be valid (id/source non-empty)
        for row in &arch.rows {
            row.validate()?;
        }
        Ok(())
    }
}

/// Build-time errors from `Builder::build`. Distinct from runtime kernel
/// errors so callers can fail fast during boot.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("row `{0}` references unknown source: {1}")]
    UnknownSource(String, String),
    #[error("row `{0}` failed validation: {1}")]
    InvalidRow(String, String),
    #[error(
        "row `{0}` (kind={1}) is a config_row; no built-in handler (Phase 5 self-extension provides the registry)"
    )]
    NoConfigRowHandler(String, String),
    #[error("row `{0}` (source={1}) has no registered factory (host plugins)")]
    NoFactory(String, String),
}

impl From<BuildError> for HarnessError {
    fn from(e: BuildError) -> Self {
        HarnessError::CompositionError(e.to_string())
    }
}

/// Factory used by [`BuilderWithFactories`] to produce a `Provider` from a
/// row whose source requires host-side dispatch (e.g. `wasm` for Extism
/// host instantiation, `pool` for hermes `_pool.yaml` compilation).
///
/// Registered at boot by the host. The kernel itself never knows how to
/// build a WASM or pool provider — it only knows the factory exists when
/// one is plugged in via `register_factory(source, factory)`.
pub type ProviderFactory = std::sync::Arc<
    dyn Fn(crate::composition::ArchitectureRow) -> Result<std::sync::Arc<dyn crate::Provider>, BuildError>
        + Send
        + Sync,
>;

/// Builder with pluggable source factories. Default [`Builder::build`] is
/// the no-factory path that only handles `native`/`wasm`/`config_row`
/// rows; [`BuilderWithFactories::build_with`] lets the host plug in
/// dispatchers for `wasm` and `pool` rows.
pub struct BuilderWithFactories {
    factories: HashMap<String, ProviderFactory>,
}

impl BuilderWithFactories {
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// Register a factory for a source string. Multiple calls for the same
    /// source replace the prior factory (last writer wins).
    pub fn register_factory(&mut self, source: impl Into<String>, factory: ProviderFactory) {
        self.factories.insert(source.into(), factory);
    }

    /// Build providers for the active rows of `arch`, consulting
    /// `factories` for sources that the built-in `Builder` does not know
    /// how to handle (`wasm`/`pool`).
    pub fn build_with(
        &self,
        arch: &Architecture,
    ) -> Result<Vec<std::sync::Arc<dyn crate::Provider>>, BuildError> {
        let mut providers: Vec<std::sync::Arc<dyn crate::Provider>> = Vec::new();
        for row in arch.active() {
            row.validate()
                .map_err(|e| BuildError::InvalidRow(row.id.clone(), e.to_string()))?;
            match row.source.as_str() {
                "native" | "wasm" if !self.factories.contains_key(row.source.as_str()) => {
                    // Built-in stub for sources the host did not register.
                    providers.push(std::sync::Arc::new(crate::row::NativeRowStub::new(
                        row.clone(),
                    )));
                }
                "config_row" => {
                    let kind = row.kind.as_deref().unwrap_or("");
                    match kind {
                        "disable" => {
                            tracing::info!(id = %row.id, "config_row kind=disable (no provider)");
                        }
                        "prompt.section" => {
                            // S7 — prompt.section rows are now buildable; the
                            // provider reads content/path at activate time.
                            providers.push(std::sync::Arc::new(crate::row::ConfigRowProvider::new(
                                row.clone(),
                            )));
                        }
                        _ => {
                            return Err(BuildError::NoConfigRowHandler(
                                row.id.clone(),
                                kind.to_string(),
                            ));
                        }
                    }
                }
                source_key => {
                    if let Some(factory) = self.factories.get(source_key) {
                        let provider = factory(row.clone())?;
                        providers.push(provider);
                    } else {
                        return Err(BuildError::UnknownSource(
                            row.id.clone(),
                            source_key.to_string(),
                        ));
                    }
                }
            }
        }
        Ok(providers)
    }
}

impl Default for BuilderWithFactories {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder: turns an [`Architecture`] into a list of `Arc<dyn Provider>`.
///
/// The native/wasm row sources are placeholders for phases 4/5; the
/// config_row kind "disable" is recognized here (no provider produced; the
/// row is logged and skipped) and config_row kind "prompt.section" passes
/// through as a stub (Phase 5 wires it to the prompt builder).
pub struct Builder;

impl Builder {
    /// Build a list of native provider stubs for the active rows of `arch`.
    ///
    /// Phase 3 stub: native and wasm rows produce a [`crate::provider::NoopProvider`]
    /// carrying their config so a downstream phase can dispatch on
    /// `source`. config_row rows are routed to a [`crate::row::ConfigRowProvider`]
    /// when the kind is recognized, otherwise [`BuildError::NoConfigRowHandler`].
    pub fn build(
        arch: &Architecture,
    ) -> Result<Vec<std::sync::Arc<dyn crate::Provider>>, BuildError> {
        let mut providers: Vec<std::sync::Arc<dyn crate::Provider>> = Vec::new();
        for row in arch.active() {
            row.validate()
                .map_err(|e| BuildError::InvalidRow(row.id.clone(), e.to_string()))?;
            match row.source.as_str() {
                "native" | "wasm" => {
                    providers.push(std::sync::Arc::new(crate::row::NativeRowStub::new(
                        row.clone(),
                    )));
                }
                "config_row" => {
                    let kind = row.kind.as_deref().unwrap_or("");
                    match kind {
                        "disable" => {
                            // Pure side-effect: log + skip.
                            tracing::info!(id = %row.id, "config_row kind=disable (no provider)");
                        }
                        "prompt.section" => {
                            providers.push(std::sync::Arc::new(crate::row::ConfigRowProvider::new(
                                row.clone(),
                            )));
                        }
                        _ => {
                            return Err(BuildError::NoConfigRowHandler(
                                row.id.clone(),
                                kind.to_string(),
                            ));
                        }
                    }
                }
                other => {
                    return Err(BuildError::UnknownSource(row.id.clone(), other.to_string()));
                }
            }
        }
        Ok(providers)
    }

    /// Build a name-indexed map of (id -> config_value) for rows of a given
    /// kind. Useful for "for every config_row of kind X, do Y" boot passes.
    pub fn rows_of_kind<'a>(
        arch: &'a Architecture,
        kind: &str,
    ) -> HashMap<&'a str, &'a serde_json::Value> {
        arch.active()
            .filter(|r| r.source == "config_row" && r.kind.as_deref() == Some(kind))
            .map(|r| (r.id.as_str(), &r.config))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn row(id: &str, source: &str, kind: Option<&str>) -> ArchitectureRow {
        ArchitectureRow {
            id: id.to_string(),
            source: source.to_string(),
            disabled: false,
            config: serde_json::json!({}),
            kind: kind.map(|s| s.to_string()),
        }
    }

    #[test]
    fn empty_architecture_validates() {
        let arch = Architecture::new();
        assert!(arch.is_empty());
        let resolved = Composition::resolve(arch, &[]).unwrap();
        assert!(resolved.is_empty());
    }

    #[test]
    fn disable_marks_row_disabled() {
        let base = Architecture {
            rows: vec![row("a", "native", None), row("b", "native", None)],
        };
        let patch = Patch {
            disable: vec![PatchTarget { id: "a".into() }],
            ..Default::default()
        };
        let resolved = Composition::resolve(base, &[patch]).unwrap();
        assert!(resolved.find("a").unwrap().disabled);
        assert!(!resolved.find("b").unwrap().disabled);
        assert_eq!(resolved.active().count(), 1);
    }

    #[test]
    fn disable_unknown_id_fails_without_mutation() {
        let base = Architecture {
            rows: vec![row("a", "native", None)],
        };
        let original_a = base.find("a").unwrap().clone();
        let patch = Patch {
            disable: vec![PatchTarget {
                id: "missing".into(),
            }],
            ..Default::default()
        };
        let err = Composition::resolve(base.clone(), &[patch]).unwrap_err();
        assert!(err.to_string().contains("missing"));
        // The original is owned by the caller; the resolver's `arch` is a
        // move-typed parameter, so the caller's copy of `base` (the one we
        // cloned into the test) is still intact.
        assert!(!base.find("a").unwrap().disabled);
        assert_eq!(base.find("a"), Some(&original_a));
    }

    #[test]
    fn replace_preserves_disabled_flag() {
        let mut r = row("a", "native", None);
        r.disabled = true;
        let base = Architecture { rows: vec![r] };
        let mut replacement = row("a", "wasm", None);
        replacement.disabled = false; // would falsely enable if we copied it
        let patch = Patch {
            replace: vec![replacement],
            ..Default::default()
        };
        let resolved = Composition::resolve(base, &[patch]).unwrap();
        let after = resolved.find("a").unwrap();
        assert_eq!(after.source, "wasm");
        assert!(after.disabled, "replace must not silently re-enable a row");
    }

    #[test]
    fn insert_then_replace_errors() {
        let base = Architecture {
            rows: vec![row("a", "native", None)],
        };
        let patch = Patch {
            insert: vec![row("a", "native", None)],
            ..Default::default()
        };
        let err = Composition::resolve(base, &[patch]).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn patches_apply_in_order() {
        let base = Architecture { rows: vec![] };
        let patches = vec![
            Patch {
                insert: vec![row("a", "native", None)],
                ..Default::default()
            },
            Patch {
                disable: vec![PatchTarget { id: "a".into() }],
                ..Default::default()
            },
            Patch {
                replace: vec![row("a", "wasm", None)],
                ..Default::default()
            },
        ];
        let resolved = Composition::resolve(base, &patches).unwrap();
        let a = resolved.find("a").unwrap();
        assert_eq!(a.source, "wasm");
        assert!(a.disabled);
    }

    #[test]
    fn config_row_requires_kind() {
        let base = Architecture {
            rows: vec![row("x", "config_row", None)],
        };
        let err = base.find("x").unwrap().validate().unwrap_err();
        assert!(err.to_string().contains("kind"));
    }

    #[test]
    fn builder_dispatches_native_and_config_row() {
        let base = Architecture {
            rows: vec![
                row("core", "native", None),
                row("disable_old", "config_row", Some("disable")),
            ],
        };
        let providers = Builder::build(&base).unwrap();
        assert_eq!(providers.len(), 1, "disable row produces no provider");
    }

    #[test]
    fn builder_rejects_unknown_source() {
        let base = Architecture {
            rows: vec![row("weird", "mystery_source", None)],
        };
        let result = Builder::build(&base);
        match result {
            Err(BuildError::UnknownSource(_, ref s)) => {
                assert!(s.contains("mystery_source"), "got: {s}");
            }
            Err(other) => panic!("expected UnknownSource, got {other}"),
            Ok(_) => panic!("expected UnknownSource, got Ok"),
        }
    }

    #[test]
    fn from_toml_parses_and_validates() {
        let src = r#"
[[rows]]
id = "core"
source = "native"
disabled = false

[[rows]]
id = "extra_prompt"
source = "config_row"
disabled = false
kind = "prompt.section"
config = { path = "/etc/prompt.md" }
"#;
        let arch = Architecture::from_toml(src).unwrap();
        assert_eq!(arch.rows.len(), 2);
        arch.validate().unwrap();
        assert_eq!(arch.rows[0].id, "core");
        assert_eq!(arch.rows[1].id, "extra_prompt");
        assert_eq!(arch.rows[1].kind.as_deref(), Some("prompt.section"));
    }

    #[test]
    fn from_toml_rejects_duplicate_ids() {
        let src = r#"
[[rows]]
id = "dup"
source = "native"

[[rows]]
id = "dup"
source = "native"
"#;
        let arch = Architecture::from_toml(src).unwrap();
        let err = arch.validate().unwrap_err();
        assert!(err.to_string().contains("duplicate"));
    }

    #[test]
    fn from_toml_rejects_malformed() {
        let src = "this is not = valid toml === [[[";
        let err = Architecture::from_toml(src).unwrap_err();
        assert!(err.to_string().contains("toml parse"));
    }
}
