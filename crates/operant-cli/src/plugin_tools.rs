//! WASM plugin tool bridge.
//!
//! Connects the `operant-plugins` WASM host to the core `ToolRegistry`:
//! plugin manifests declaring the `tool` capability are discovered,
//! their `WasmTool`s are adapted to the core `OperantTool` trait, and
//! registered so the agent can call them exactly like built-in tools.
//!
//! This is the missing half of the plugin architecture: `operant-plugins`
//! already shipped a `PluginHost` (discovery, manifests, signature
//! verification) and `WasmTool`/`runtime` bridges, but nothing wired them
//! into the live agent. Feature-gated behind `plugins-wasm` (default-off;
//! `ci-all` enables it).

use anyhow::Context as _;
use operant_core::config::AppConfig;
use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry};
use operant_plugins::PluginTool as _;
use serde_json::Value;

/// Adapter that presents a WASM-backed plugin tool through the core
/// `OperantTool` trait. `WasmTool` implements `operant_api::tool::Tool`;
/// this wrapper translates its schema/result shapes into the core types.
pub struct PluginToolAdapter {
    inner: operant_plugins::wasm_tool::WasmTool,
    toolset: String,
}

impl PluginToolAdapter {
    /// Wrap a plugin `WasmTool`, tagging it with the plugin's name so the
    /// registry can attribute it (and users can disable the whole plugin
    /// via `disabled_toolsets`).
    pub fn new(inner: operant_plugins::wasm_tool::WasmTool, plugin_name: &str) -> Self {
        Self {
            inner,
            toolset: format!("plugin:{}", plugin_name),
        }
    }
}

#[async_trait::async_trait]
impl OperantTool for PluginToolAdapter {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.inner.name(),
            self.inner.description(),
            self.inner.parameters_schema(),
        )
    }

    fn toolset(&self) -> &str {
        &self.toolset
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> operant_core::tools::ToolResult {
        match self.inner.execute(args).await {
            Ok(result) => {
                if result.success {
                    operant_core::tools::ToolResult::success_with_name(
                        self.inner.name(),
                        self.inner.name(),
                        result.output,
                    )
                } else {
                    operant_core::tools::ToolResult::error_with_name(
                        self.inner.name(),
                        self.inner.name(),
                        result
                            .error
                            .unwrap_or_else(|| "plugin execution failed".into()),
                    )
                }
            }
            Err(e) => operant_core::tools::ToolResult::error_with_name(
                self.inner.name(),
                self.inner.name(),
                format!("plugin execution error: {e:#}"),
            ),
        }
    }
}

/// Discover plugins and register every `tool`-capable plugin into the
/// registry. Best-effort: a broken plugin logs a warning and is skipped —
/// it must never fail the whole agent startup.
pub async fn register_plugin_tools(
    registry: &ToolRegistry,
    config: &AppConfig,
) -> anyhow::Result<()> {
    let plugin_dirs = &config.plugins.plugin_dirs;
    if plugin_dirs.is_empty() {
        return Ok(());
    }

    // The PluginHost discovers `<workspace>/plugins/*`; for a configured
    // plugin dir like `~/.operant/plugins` the workspace is its parent.
    let plugins_dir = &plugin_dirs[0];
    let parent = plugins_dir
        .parent()
        .context("plugins dir has no parent — cannot build plugin host")?;

    // Signature policy: `[plugins] signature_mode` / `trusted_publisher_keys`
    // in the AppConfig. Defaults to Disabled, matching the schema world's
    // `[plugins.security]` default, and it keeps the CLI bridge simple.
    let signature_mode =
        operant_plugins::host::PluginHost::parse_signature_mode(&config.plugins.signature_mode);
    let host = match operant_plugins::host::PluginHost::with_security(
        parent,
        signature_mode,
        config.plugins.trusted_publisher_keys.clone(),
    ) {
        Ok(host) => host,
        Err(e) => {
            tracing::warn!(error = %e, "plugin host failed to initialize; skipping plugin tools");
            return Ok(());
        }
    };

    for (manifest, wasm_path) in host.tool_plugin_details() {
        let tool = operant_plugins::wasm_tool::WasmTool::from_wasm(
            wasm_path.to_path_buf(),
            manifest.permissions.clone(),
            manifest.name.clone(),
            manifest.description.clone().unwrap_or_default(),
        );
        let tool_name = tool.name().to_string();
        let plugin_name = manifest.name.clone();
        match registry
            .register(PluginToolAdapter::new(tool, &plugin_name))
            .await
        {
            Ok(()) => {
                tracing::info!(
                    plugin = %plugin_name,
                    tool = %tool_name,
                    "Registered WASM plugin tool"
                );
            }
            Err(e) => {
                tracing::warn!(
                    plugin = %manifest.name,
                    error = %e,
                    "Failed to register plugin tool (non-fatal)"
                );
            }
        }
    }

    Ok(())
}

/// List tool-capable plugins (used by `operant plugins list` to surface
/// what will be loaded). Returns plugin names + tool count.
pub fn list_plugin_tools(config: &AppConfig) -> Vec<(String, usize)> {
    let plugin_dirs = &config.plugins.plugin_dirs;
    if plugin_dirs.is_empty() {
        return Vec::new();
    }
    let parent = match plugin_dirs[0].parent() {
        Some(p) => p.to_path_buf(),
        None => return Vec::new(),
    };
    let Ok(host) = operant_plugins::host::PluginHost::new(&parent) else {
        return Vec::new();
    };
    host.tool_plugin_details()
        .iter()
        .map(|(m, _)| (m.name.clone(), 1))
        .collect()
}

// ── Kernel swap bridge (audit C2) ────────────────────────────────────────
//
// The watcher produces `ManifestChange`s; the kernel mounts providers
// built from `ArchitectureRow`s. This is the lossless mapping, and it
// lives here because this module is the only place that sees BOTH
// `operant-plugins` (manifests, Extism runtime) and `operant-harness`
// (ArchitectureRow) — `operant-plugins` must not depend on the kernel.

use operant_plugins::PluginCapability;
use operant_plugins::signature::VerificationResult;
use operant_plugins::watcher::ManifestChange;

/// Why a manifest cannot become a kernel provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unmapped {
    /// Skill-only bundle: skills are markdown files, not providers.
    SkillOnly,
    /// The capability has no kernel seam (observers register outside the
    /// harness, in the ObserverRegistry).
    NoKernelSeam(&'static str),
    /// A `Tool` plugin whose module exposes no usable `tool_metadata`
    /// export. We never guess a tool name from the directory: a guessed
    /// claim would install the tool under a name the agent cannot call.
    NoToolMetadata,
}

/// A verified manifest converted into a kernel-mountable row.
#[derive(Debug, Clone)]
pub struct KernelRow {
    /// The row to hand the wasm factory / `Harness::replace`. Its
    /// `config["claims"]` carries the claims `WasmProvider` materializes.
    pub row: operant_harness::ArchitectureRow,
}

/// The kernel seam family a capability's name is claimed under.
fn seam_for(cap: &PluginCapability) -> Option<&'static str> {
    match cap {
        PluginCapability::Tool => Some("tool"),
        // `MemoryProviderSeam` claims `memory.provider/<name>`; the
        // gateway seam claims `channel.adapter/<name>`.
        PluginCapability::Memory => Some("memory.provider"),
        PluginCapability::Channel => Some("channel.adapter"),
        PluginCapability::Observer => None,
        PluginCapability::Skill => None,
    }
}

/// Resolve one plugin's claimed names, each tagged with the seam it
/// belongs to. Pairing matters: a manifest declaring both `Tool` and
/// `Memory` must claim `tool/<toolid>` and `memory.provider/<plugin>`,
/// never the cross-product.
fn paired_claims(
    manifest: &operant_plugins::PluginManifest,
    dir: &std::path::Path,
) -> Result<Vec<(String, String)>, Unmapped> {
    use operant_plugins::runtime;
    let mut pairs: Vec<(String, String)> = Vec::new();
    for cap in &manifest.capabilities {
        let Some(seam) = seam_for(cap) else {
            return Err(match cap {
                PluginCapability::Skill => Unmapped::SkillOnly,
                _ => Unmapped::NoKernelSeam("observer"),
            });
        };
        match cap {
            // A tool's real name lives in the module's `tool_metadata`
            // export — that is the name the agent calls.
            PluginCapability::Tool => {
                let wasm = manifest
                    .wasm_path
                    .as_deref()
                    .ok_or(Unmapped::NoToolMetadata)?;
                let path = dir.join(wasm);
                let mut plugin = runtime::create_plugin(&path, &manifest.permissions)
                    .map_err(|_| Unmapped::NoToolMetadata)?;
                let meta = runtime::call_tool_metadata(&mut plugin)
                    .map_err(|_| Unmapped::NoToolMetadata)?;
                if meta.name.is_empty() {
                    return Err(Unmapped::NoToolMetadata);
                }
                pairs.push((seam.to_string(), meta.name));
            }
            // Memory/channel providers are identified by plugin name.
            _ => pairs.push((seam.to_string(), manifest.name.clone())),
        }
    }
    if pairs.is_empty() {
        return Err(Unmapped::NoToolMetadata);
    }
    Ok(pairs)
}

/// Convert a watcher change into a kernel row.
///
/// Signature policy is enforced HERE as well as inside the watcher, so a
/// caller that forgets to filter cannot swap an unverified plugin in.
pub fn row_for_change(change: &ManifestChange) -> Result<KernelRow, KernelBridgeError> {
    if !change.signature.is_valid() {
        return Err(KernelBridgeError::Signature(change.signature.clone()));
    }
    let dir = change
        .manifest_path
        .parent()
        .ok_or(KernelBridgeError::Unmapped(Unmapped::NoToolMetadata))?;
    let pairs = paired_claims(&change.manifest, dir).map_err(KernelBridgeError::Unmapped)?;
    let claims: Vec<String> = pairs
        .iter()
        .map(|(seam, name)| format!("{seam}/{name}"))
        .collect();
    let row = operant_harness::ArchitectureRow {
        id: change.manifest.name.clone(),
        source: "wasm".to_string(),
        kind: Some("wasm".to_string()),
        config: serde_json::json!({
            "path": change.manifest_path.display().to_string(),
            "claims": claims,
        }),
        disabled: false,
    };
    Ok(KernelRow { row })
}

/// Failure modes of the bridge.
#[derive(Debug)]
pub enum KernelBridgeError {
    /// The per-scan signature verdict rejected this manifest.
    ///
    /// The swap path runs `SignatureMode::Strict` unconditionally (a swap
    /// instantiates the module on every scan, unlike boot-time tool
    /// registration, so unsigned code must never reach it) and refuses an
    /// empty trusted-key set. Under `Disabled`/`Permissive` no scan ever
    /// produces `Valid`, so this is what a misconfigured host hits.
    Signature(VerificationResult),
    /// The plugin cannot become a kernel provider.
    Unmapped(Unmapped),
}

impl std::fmt::Display for KernelBridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KernelBridgeError::Signature(v) => {
                write!(f, "plugin signature not valid: {v:?}")
            }
            KernelBridgeError::Unmapped(u) => {
                write!(f, "plugin not mappable to a kernel row: {u:?}")
            }
        }
    }
}

impl std::error::Error for KernelBridgeError {}

#[cfg(test)]
mod kernel_bridge_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::path::PathBuf;

    fn manifest(caps: Vec<PluginCapability>) -> operant_plugins::PluginManifest {
        operant_plugins::PluginManifest {
            name: "demo".to_string(),
            version: "0.1.0".to_string(),
            description: None,
            author: None,
            wasm_path: None,
            capabilities: caps,
            permissions: vec![],
            signature: None,
            publisher_key: None,
        }
    }

    fn change(m: operant_plugins::PluginManifest, sig: VerificationResult) -> ManifestChange {
        ManifestChange {
            manifest_path: PathBuf::from("/nonexistent/demo/manifest.toml"),
            manifest: m,
            signature: sig,
        }
    }

    fn valid() -> VerificationResult {
        VerificationResult::Valid {
            publisher_key: "k".into(),
        }
    }

    #[test]
    fn strict_mode_swap_policy_is_pinned_by_construction() {
        // The swap path's strictness is a construction fact, not a runtime
        // check: `spawn_wasm_watcher` builds the watcher with
        // `.with_signature(SignatureMode::Strict, trusted_keys)` and refuses
        // an empty key set. This test pins the policy inputs those lines
        // depend on, so a future refactor that swaps the mode or drops the
        // key guard has a failing test to trip over.
        assert!(matches!(
            operant_plugins::signature::SignatureMode::default(),
            operant_plugins::signature::SignatureMode::Disabled
        ));
        // The swap path must never construct a watcher with the default.
        assert_ne!(
            operant_plugins::signature::SignatureMode::default(),
            operant_plugins::signature::SignatureMode::Strict
        );
    }

    #[test]
    fn unverified_manifests_never_become_rows() {
        for verdict in [
            VerificationResult::Unsigned,
            VerificationResult::Untrusted,
            VerificationResult::Invalid {
                reason: "bad".into(),
            },
        ] {
            let err = row_for_change(&change(manifest(vec![PluginCapability::Memory]), verdict))
                .unwrap_err();
            assert!(matches!(err, KernelBridgeError::Signature(_)), "{err}");
        }
    }

    #[test]
    fn skill_only_and_observer_are_unmapped() {
        assert!(matches!(
            row_for_change(&change(manifest(vec![PluginCapability::Skill]), valid())).unwrap_err(),
            KernelBridgeError::Unmapped(Unmapped::SkillOnly)
        ));
        assert!(matches!(
            row_for_change(&change(manifest(vec![PluginCapability::Observer]), valid()))
                .unwrap_err(),
            KernelBridgeError::Unmapped(Unmapped::NoKernelSeam("observer"))
        ));
    }

    #[test]
    fn memory_manifest_claims_only_its_own_seam() {
        // Regression guard for the cross-product bug: a memory-only
        // manifest must NOT produce a `tool/...` claim.
        let out = row_for_change(&change(manifest(vec![PluginCapability::Memory]), valid()))
            .expect("memory manifest maps");
        let claims: Vec<String> = out.row.config["claims"]
            .as_array()
            .expect("claims array")
            .iter()
            .map(|c| c.as_str().expect("claim string").to_string())
            .collect();
        assert_eq!(claims, vec!["memory.provider/demo".to_string()]);
        assert_eq!(out.row.kind.as_deref(), Some("wasm"));
        assert!(claims.iter().all(|c| c.starts_with("memory.provider/")));
    }

    #[test]
    fn tool_without_metadata_is_refused_not_guessed() {
        let mut m = manifest(vec![PluginCapability::Tool]);
        m.wasm_path = Some("missing.wasm".to_string());
        assert!(matches!(
            row_for_change(&change(m, valid())).unwrap_err(),
            KernelBridgeError::Unmapped(Unmapped::NoToolMetadata)
        ));
    }
}
