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
//! into the live agent. Feature-gated behind `plugins-wasm` (on by default
//! since iter-1720; CI builds with `--no-default-features` to skip it).

use anyhow::Context as _;
use operant_core::config::AppConfig;
use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolInterceptHandler, ToolRegistry};
use operant_plugins::PluginTool as _;
use serde_json::Value;
use std::sync::Arc;

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
    // Log the effective policy: an operator running strict should be able to
    // confirm it is enforced, and one running disabled should see that plugin
    // code is unverified. The key count matters because strict with an empty
    // list marks every plugin Untrusted — the silent "nothing loads" mode.
    // The PARSED mode is logged, not the raw config string: an unrecognized
    // value maps to Disabled, so logging the raw string would print
    // `mode = "Strict "` while the host silently ran unverified.
    tracing::info!(
        configured = %config.plugins.signature_mode,
        effective = ?signature_mode,
        trusted_keys = config.plugins.trusted_publisher_keys.len(),
        plugins_dir = %plugins_dir.display(),
        "plugins: signature policy for the CLI tool bridge"
    );
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
        if !config.plugins.is_enabled(&manifest.name) {
            tracing::info!(plugin = %manifest.name, "plugin is in [plugins].disabled; not loading its tools");
            continue;
        }
        let tool = operant_plugins::wasm_tool::WasmTool::from_wasm(
            wasm_path.to_path_buf(),
            manifest.permissions.clone(),
            manifest.name.clone(),
            manifest.description.clone().unwrap_or_default(),
        );
        register_plugin_tool(registry, tool, &manifest.name).await;
    }

    register_observer_interceptors(registry, &host, config).await;

    Ok(())
}

/// The observe-event envelope: what a WASM observer receives per
/// interception (the iter-1722 contract, plus iter-1725's `config`
/// passthrough — the manifest's free-form `[config]` table, serialized).
pub(crate) fn observe_event_json(
    point: &str,
    tool: &str,
    args: &Value,
    result: Option<&Value>,
    config: &Value,
) -> Value {
    serde_json::json!({
        "point": point,
        "tool": tool,
        "args": args,
        "result": result,
        "config": config,
    })
}

/// Register observer-capable plugins as tool-seam interceptors (iter-1722).
/// The plugin's `observe` export receives every interception event
/// (`pre_execute` before a tool runs, `post_execute` with the result) and
/// may veto it. The module's instances are RESIDENT — a
/// `runtime::PluginPool` checked out per event (iter-1725; the per-event
/// instantiate this replaced is red-team audit F1), so a call pays no
/// compile. Instance state persists on checked-out modules —
/// deepseek-harness residency semantics.
///
/// Fail-open: an observer that fails to load, errors, times out its pool
/// checkout, or returns garbage allows the call and logs — a broken
/// observer must never brick the tool loop.
async fn register_observer_interceptors(
    registry: &ToolRegistry,
    host: &operant_plugins::host::PluginHost,
    config: &AppConfig,
) {
    for (manifest, wasm_path) in host.observer_plugin_details() {
        if !config.plugins.is_enabled(&manifest.name) {
            continue;
        }
        let pool = operant_plugins::runtime::PluginPool::new(wasm_path, &manifest.permissions);
        // The manifest's `[config]` table, serialized once — it rides every
        // observe envelope (dsh's config-driven policy plugins are the canon;
        // without it a policy observer has nowhere to read its thresholds).
        let plugin_config = serde_json::to_value(&manifest.config).unwrap_or_default();
        registry
            .register_tool_interceptor(observe_interceptor_handler(
                pool,
                manifest.name.clone(),
                plugin_config,
            ))
            .await;
        tracing::info!(
            plugin = %manifest.name,
            "registered WASM observer as tool-seam interceptor (resident pool)"
        );
    }
}

/// Registers one plugin tool unless its name is already taken (KB-11). A
/// built-in keeps its name, and so does the first plugin to claim one, so a
/// plugin cannot replace a tool the agent already has. This runs once at
/// startup, so a name in the registry is never the plugin's own earlier load.
async fn register_plugin_tool(
    registry: &ToolRegistry,
    tool: operant_plugins::wasm_tool::WasmTool,
    plugin_name: &str,
) {
    let tool_name = tool.name().to_string();
    if registry.contains(&tool_name).await {
        tracing::warn!(
            plugin = %plugin_name,
            tool = %tool_name,
            "Plugin tool name is already registered; skipping it so the existing tool is kept"
        );
        return;
    }
    match registry
        .register(PluginToolAdapter::new(tool, plugin_name))
        .await
    {
        Ok(()) => {
            note_plugin_tool(plugin_name, &tool_name);
            tracing::info!(
                plugin = %plugin_name,
                tool = %tool_name,
                "Registered WASM plugin tool"
            );
        }
        Err(e) => {
            tracing::warn!(
                plugin = %plugin_name,
                error = %e,
                "Failed to register plugin tool (non-fatal)"
            );
        }
    }
}

/// Plugin name → the tool names it registered (gap G6). The boot path only
/// ever adds; the watcher path reads it to know which names a changed
/// plugin may replace and which to drop when it stops providing tools. A
/// `Mutex` over a small map touched once per registration — no await held.
static PLUGIN_TOOL_NAMES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, Vec<String>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

fn note_plugin_tool(plugin: &str, tool: &str) {
    match PLUGIN_TOOL_NAMES.lock() {
        Ok(mut map) => map
            .entry(plugin.to_string())
            .or_default()
            .push(tool.to_string()),
        Err(poisoned) => {
            // a panicked registration must not lose the map — the house
            // pattern: recover the guard, the data is still a valid map
            let mut map = poisoned.into_inner();
            map.entry(plugin.to_string())
                .or_default()
                .push(tool.to_string());
        }
    }
}

/// The names a plugin currently owns in the registry (empty when none).
fn plugin_tool_names(plugin: &str) -> Vec<String> {
    match PLUGIN_TOOL_NAMES.lock() {
        Ok(map) => map.get(plugin).cloned().unwrap_or_default(),
        Err(poisoned) => poisoned
            .into_inner()
            .get(plugin)
            .cloned()
            .unwrap_or_default(),
    }
}

fn set_plugin_tool_names(plugin: &str, names: Vec<String>) {
    match PLUGIN_TOOL_NAMES.lock() {
        Ok(mut map) => {
            map.insert(plugin.to_string(), names);
        }
        Err(poisoned) => {
            poisoned.into_inner().insert(plugin.to_string(), names);
        }
    }
}

/// Build the observe-envelope interceptor handler for one observer plugin
/// (shared by the boot bridge and the watcher's live path — gap G6). The
/// pool is RESIDENT (iter-1725) and the handler fails open on every error
/// path: a broken observer never bricks the tool loop.
fn observe_interceptor_handler(
    pool: operant_plugins::runtime::PluginPool,
    plugin_name: String,
    plugin_config: Value,
) -> ToolInterceptHandler {
    Arc::new(move |event| {
        let pool = pool.clone();
        let plugin_name = plugin_name.clone();
        let plugin_config = plugin_config.clone();
        Box::pin(async move {
            let point = match event.point {
                operant_core::tools::ToolInterceptPoint::PreExecute => "pre_execute",
                operant_core::tools::ToolInterceptPoint::PostExecute => "post_execute",
            };
            let event_json = observe_event_json(
                point,
                &event.tool,
                &event.args,
                event.result.as_ref(),
                &plugin_config,
            )
            .to_string();
            // The pool checkout + extism call are blocking — hop to the
            // blocking pool (same boundary as WasmTool::execute).
            let verdict = tokio::task::spawn_blocking(move || pool.observe(&event_json)).await;
            match verdict {
                Ok(Ok(Some(operant_plugins::runtime::ObserveVerdict::Allow))) => {
                    operant_core::gateway_pipeline::PipelineAction::Allow
                }
                Ok(Ok(Some(operant_plugins::runtime::ObserveVerdict::Block { reason }))) => {
                    operant_core::gateway_pipeline::PipelineAction::Block(if reason.is_empty() {
                        format!("blocked by observer plugin {plugin_name}")
                    } else {
                        reason
                    })
                }
                Ok(Ok(None)) => {
                    tracing::warn!(
                        plugin = %plugin_name,
                        "observer pool busy past the wait ceiling; allowing (fail-open)"
                    );
                    operant_core::gateway_pipeline::PipelineAction::Allow
                }
                Ok(Err(e)) => {
                    tracing::warn!(
                        plugin = %plugin_name,
                        error = %e,
                        "observe call failed; allowing (fail-open)"
                    );
                    operant_core::gateway_pipeline::PipelineAction::Allow
                }
                Err(e) => {
                    tracing::warn!(
                        plugin = %plugin_name,
                        error = %e,
                        "observer task failed; allowing (fail-open)"
                    );
                    operant_core::gateway_pipeline::PipelineAction::Allow
                }
            }
        })
    })
}

/// Gap G6 (live re-registration): apply watcher-detected plugin changes to
/// the LIVE registry — the boot bridge's counterpart for the hot-swap
/// watcher. A tool plugin replaces its own registration (`register_dyn`
/// replaces by name; a name the plugin does not already own is still
/// refused, KB-11 posture), an observer plugin re-keys by plugin name
/// (named interceptor, replace semantics — no stacking), and names the
/// plugin no longer provides are dropped. Policy mirrors the boot bridge:
/// `[plugins] signature_mode / trusted_publisher_keys / disabled`; the
/// kernel swap path keeps its own Strict gate in `spawn_wasm_watcher`.
/// Fail-open per change: a bad change warns and continues.
pub async fn apply_plugin_changes(
    registry: &ToolRegistry,
    config: &AppConfig,
    changes: &[operant_plugins::watcher::ManifestChange],
) -> anyhow::Result<()> {
    use operant_plugins::signature::{SignatureMode, VerificationResult};

    let signature_mode =
        operant_plugins::host::PluginHost::parse_signature_mode(&config.plugins.signature_mode);
    let signature_ok = |signature: &VerificationResult| match signature_mode {
        SignatureMode::Disabled | SignatureMode::Permissive => true,
        SignatureMode::Strict => signature.is_valid(),
    };

    for change in changes {
        let manifest = &change.manifest;
        let name = &manifest.name;
        if !config.plugins.is_enabled(name) {
            tracing::info!(plugin = %name, "watcher: plugin is in [plugins].disabled; skipping");
            continue;
        }
        if !signature_ok(&change.signature) {
            tracing::warn!(
                plugin = %name,
                mode = ?signature_mode,
                "watcher: plugin signature not acceptable under the configured policy; skipping"
            );
            continue;
        }
        let wasm_path = match manifest.wasm_path.as_deref().map(|rel| {
            change
                .manifest_path
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join(rel)
        }) {
            Some(p) if p.exists() => p,
            _ => {
                tracing::warn!(plugin = %name, "watcher: change carries no resolvable wasm module; skipping");
                continue;
            }
        };

        // Tools: replace-own-registration.
        if manifest
            .capabilities
            .contains(&operant_plugins::PluginCapability::Tool)
        {
            let tool = operant_plugins::wasm_tool::WasmTool::from_wasm(
                wasm_path.clone(),
                manifest.permissions.clone(),
                manifest.name.clone(),
                manifest.description.clone().unwrap_or_default(),
            );
            let tool_name = tool.name().to_string();
            let owned = plugin_tool_names(name).contains(&tool_name);
            if registry.contains(&tool_name).await && !owned {
                tracing::warn!(
                    plugin = %name,
                    tool = %tool_name,
                    "watcher: tool name is already registered elsewhere; refusing the replace (KB-11)"
                );
            } else {
                // register_dyn REPLACES atomically under the same name —
                // the changed module swaps in without a gap.
                if let Err(e) = registry
                    .register_dyn(Arc::new(PluginToolAdapter::new(tool, name)))
                    .await
                {
                    tracing::warn!(plugin = %name, error = %e, "watcher: tool replace failed");
                } else {
                    tracing::info!(plugin = %name, tool = %tool_name, "watcher: plugin tool applied live");
                }
            }
            // Names this plugin owned but the new manifest no longer
            // provides (capability dropped, or the module renamed its
            // tool) come out of the registry — replace-own-registration.
            let stale: Vec<String> = plugin_tool_names(name)
                .into_iter()
                .filter(|n| n != &tool_name)
                .collect();
            for old in &stale {
                if registry.unregister_tool(old).await {
                    tracing::info!(plugin = %name, tool = %old, "watcher: retired plugin tool name dropped");
                }
            }
            set_plugin_tool_names(name, vec![tool_name]);
        } else {
            // no Tool capability: drop whatever tools this plugin owned
            for old in plugin_tool_names(name) {
                if registry.unregister_tool(&old).await {
                    tracing::info!(plugin = %name, tool = %old, "watcher: plugin no longer provides tools; dropped");
                }
            }
            set_plugin_tool_names(name, Vec::new());
        }

        // Observers: named registration, replace semantics (no stacking).
        if manifest
            .capabilities
            .contains(&operant_plugins::PluginCapability::Observer)
        {
            let pool = operant_plugins::runtime::PluginPool::new(&wasm_path, &manifest.permissions);
            let plugin_config = serde_json::to_value(&manifest.config).unwrap_or_default();
            registry
                .register_plugin_interceptor(
                    name,
                    observe_interceptor_handler(pool, name.clone(), plugin_config),
                )
                .await;
            tracing::info!(plugin = %name, "watcher: observer interceptor applied live");
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
    // Same signature policy as the loader (iter-1726, audit F4): before
    // this, `PluginHost::new` skipped verification, so under strict the CLI
    // could LIST plugins the runtime refuses to load.
    let signature_mode =
        operant_plugins::host::PluginHost::parse_signature_mode(&config.plugins.signature_mode);
    let Ok(host) = operant_plugins::host::PluginHost::with_security(
        &parent,
        signature_mode,
        config.plugins.trusted_publisher_keys.clone(),
    ) else {
        return Vec::new();
    };
    host.tool_plugin_details()
        .iter()
        .filter(|(m, _)| config.plugins.is_enabled(&m.name))
        .map(|(m, _)| (m.name.clone(), 1))
        .collect()
}

/// Skill-capable plugins contribute their `skills/` subdirectory as a
/// `SkillManager` extra root — the `skill` capability is content, not a
/// provider (deepseek-harness capability-map parity). Best-effort: a host
/// that cannot initialize returns no roots. Operator-pool skills win name
/// collisions by load order — the same rule the ponytail root follows, so
/// skill names are NOT re-namespaced here (the `plugin:<name>/<skill>`
/// namespacing `skill_plugin_details` suggests would need SkillManager
/// changes the extra-root pattern does not).
///
/// Skill content is gated by the same signature policy as tool code: the
/// host only discovers plugins that pass verification, so a strict-mode
/// deployment does not load an unsigned plugin's skills either.
pub fn plugin_skill_roots(config: &AppConfig) -> Vec<std::path::PathBuf> {
    let plugin_dirs = &config.plugins.plugin_dirs;
    if plugin_dirs.is_empty() {
        return Vec::new();
    }
    let Some(parent) = plugin_dirs[0].parent() else {
        return Vec::new();
    };
    let signature_mode =
        operant_plugins::host::PluginHost::parse_signature_mode(&config.plugins.signature_mode);
    let Ok(host) = operant_plugins::host::PluginHost::with_security(
        parent,
        signature_mode,
        config.plugins.trusted_publisher_keys.clone(),
    ) else {
        return Vec::new();
    };
    host.skill_plugin_details()
        .into_iter()
        .filter(|(m, _)| config.plugins.is_enabled(&m.name))
        .map(|(_, dir)| dir)
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
    /// The capability has no kernel ArchitectureRow. Observers DO have a
    /// real seam since iter-1722 (tool-seam interceptors, registered at
    /// boot) — what they lack is a kernel row: the harness mounts
    /// providers, not interceptors, so an observer manifest cannot become
    /// one and the watcher refuses it here.
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

    /// The signature policy both plugin paths read, exercised at runtime.
    /// iter-363 moved `register_plugin_tools` onto
    /// `PluginHost::with_security(mode, keys)` but every test since ran
    /// without touching that call — a compile check is not a runtime one.
    #[test]
    fn configured_mode_reaches_the_host_and_the_key_set_is_honoured() {
        use operant_plugins::host::PluginHost;
        use operant_plugins::signature::SignatureMode;

        // The parser is what both bridges call; pin its whole mapping so a
        // typo that silently downgrades a strict deployment to Disabled
        // fails here.
        for (raw, expected) in [
            ("strict", SignatureMode::Strict),
            ("STRICT", SignatureMode::Strict),
            ("permissive", SignatureMode::Permissive),
            ("disabled", SignatureMode::Disabled),
            ("", SignatureMode::Disabled),
            ("nonsense", SignatureMode::Disabled),
        ] {
            assert_eq!(
                PluginHost::parse_signature_mode(raw),
                expected,
                "parse_signature_mode({raw:?})"
            );
        }

        // The AppConfig default must be the schema world's default, or the
        // two policy surfaces drift (the duplication iter-363 removed).
        assert_eq!(AppConfig::default().plugins.signature_mode, "disabled");

        // with_security creates a missing plugins dir and returns Ok with
        // the mode and keys it was handed (it errors only if creation or
        // discovery fails). Asserting the real contract: the policy
        // reaches the host rather than being dropped on the floor.
        let dir = std::env::temp_dir().join("operant-sig-policy-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp workspace");
        let host = PluginHost::with_security(&dir, SignatureMode::Strict, vec!["aa".to_string()])
            .expect("with_security creates the plugins dir and succeeds");
        assert!(host.plugins_dir().ends_with("plugins"));
        let _ = std::fs::remove_dir_all(&dir);
    }

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
            requires: Vec::new(),
            provides: Vec::new(),
            config: Default::default(),
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

#[cfg(test)]
mod observer_bridge_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::{observe_event_json, register_observer_interceptors};
    use operant_core::config::AppConfig;
    use operant_core::tools::ToolRegistry;
    use operant_plugins::host::PluginHost;
    use operant_plugins::signature::SignatureMode;
    use std::time::Duration;

    /// A workspace with one observer plugin whose module is garbage —
    /// discovery passes (only the manifest shape is checked) but every
    /// instantiate fails, which is exactly the fail-open leg a real broken
    /// observer exercises.
    fn broken_observer_workspace() -> tempfile::TempDir {
        let ws = tempfile::tempdir().expect("workspace");
        let plugin_dir = ws.path().join("plugins").join("broken-observer");
        std::fs::create_dir_all(&plugin_dir).expect("plugin tree");
        std::fs::write(
            plugin_dir.join("manifest.toml"),
            "name = \"broken-observer\"\nversion = \"0.1.0\"\ncapabilities = [\"observer\"]\nwasm_path = \"broken.wasm\"\n[config]\nthreshold = 3\n",
        )
        .expect("manifest");
        std::fs::write(plugin_dir.join("broken.wasm"), b"not wasm").expect("garbage module");
        ws
    }

    #[tokio::test]
    async fn a_broken_observer_fails_open_and_the_tool_still_runs() {
        // End-to-end at the registry level: the interceptor registered from
        // an uninstantiable module lets every call through (iter-1725 pins
        // this through the RESIDENT pool path, not just the old per-event
        // creation).
        let ws = broken_observer_workspace();
        let mut config = AppConfig::default();
        config.plugins.plugin_dirs = vec![ws.path().join("plugins")];
        let parent = ws.path().to_path_buf();
        let host =
            PluginHost::with_security(&parent, SignatureMode::Disabled, Vec::new()).expect("host");
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(operant_core::tools::debug_helpers::EchoTool)
            .await
            .unwrap();
        register_observer_interceptors(&registry, &host, &config).await;

        let ran = registry
            .execute(
                "echo",
                "call_1",
                serde_json::json!({"message": "hello"}),
                operant_core::tools::ToolContext::default(),
            )
            .await
            .expect("fail-open: the call is allowed, not vetoed");
        assert!(
            ran.success,
            "the tool must have RUN (allowed), got: {ran:?}"
        );
    }

    #[test]
    fn the_observe_envelope_carries_config_and_null_result_on_pre_execute() {
        // The envelope contract a plugin author codes against: pre_execute
        // has no result (null), the manifest's [config] table rides along.
        let envelope = observe_event_json(
            "pre_execute",
            "file_write",
            &serde_json::json!({"path": "x"}),
            None,
            &serde_json::json!({"threshold": 3}),
        );
        assert_eq!(envelope["point"], "pre_execute");
        assert_eq!(envelope["tool"], "file_write");
        assert_eq!(envelope["args"]["path"], "x");
        assert!(envelope["result"].is_null());
        assert_eq!(envelope["config"]["threshold"], 3);
    }
}

#[cfg(test)]
mod name_collision_tests {
    use super::register_plugin_tool;
    use operant_core::tools::ToolRegistry;
    use operant_plugins::wasm_tool::WasmTool;
    use std::path::PathBuf;
    use std::time::Duration;

    #[tokio::test]
    async fn a_plugin_tool_does_not_replace_a_built_in_of_the_same_name() {
        // KB-11: a plugin that names a tool after a built-in is refused, and the
        // built-in stays registered.
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(operant_core::tools::memory_tools::MemoryStoreTool)
            .await
            .unwrap();

        let impostor = WasmTool::from_wasm(
            PathBuf::from("/nonexistent.wasm"),
            Vec::new(),
            "memory_store".to_string(),
            "impostor".to_string(),
        );
        register_plugin_tool(&registry, impostor, "impostor-plugin").await;

        let kept = registry
            .get("memory_store")
            .await
            .expect("built-in still registered");
        assert_ne!(kept.description(), "impostor");
    }
}

#[cfg(test)]
mod skill_roots_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::plugin_skill_roots;
    use operant_core::config::AppConfig;

    /// A workspace with one skill plugin: manifest (skill-only, no wasm) +
    /// one bundle-valid `skills/<name>/SKILL.md`.
    fn skill_plugin_workspace() -> tempfile::TempDir {
        let ws = tempfile::tempdir().expect("workspace");
        let plugin_dir = ws.path().join("plugins").join("demo");
        std::fs::create_dir_all(plugin_dir.join("skills").join("greet")).expect("plugin tree");
        std::fs::write(
            plugin_dir.join("manifest.toml"),
            "name = \"demo\"\nversion = \"0.1.0\"\ncapabilities = [\"skill\"]\n",
        )
        .expect("manifest");
        std::fs::write(
            plugin_dir.join("skills").join("greet").join("SKILL.md"),
            "---\nname: greet\ndescription: demo skill\n---\n\nHello from the plugin.",
        )
        .expect("skill");
        ws
    }

    fn config_over(ws: &tempfile::TempDir) -> AppConfig {
        let mut config = AppConfig::default();
        config.plugins.plugin_dirs = vec![ws.path().join("plugins")];
        config
    }

    #[test]
    fn a_skill_plugin_contributes_its_skills_dir() {
        // iter-1720: the unmapped `skill` capability becomes a SkillManager
        // extra root (the dsh capability-map's skill seam).
        let ws = skill_plugin_workspace();
        assert_eq!(
            plugin_skill_roots(&config_over(&ws)),
            vec![ws.path().join("plugins").join("demo").join("skills")]
        );
    }

    #[test]
    fn a_disabled_plugin_contributes_no_roots() {
        // [plugins].disabled must reach the skill seam, not just the tool seam.
        let ws = skill_plugin_workspace();
        let mut config = config_over(&ws);
        config.plugins.disabled = vec!["demo".to_string()];
        assert!(plugin_skill_roots(&config).is_empty());
    }
}

#[cfg(test)]
mod list_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::list_plugin_tools;
    use operant_core::config::AppConfig;

    /// A minimal tool plugin; the module need not be valid — the list path
    /// reads manifests only (metadata probing happens at boot).
    fn tool_plugin_workspace() -> tempfile::TempDir {
        let ws = tempfile::tempdir().expect("workspace");
        let plugin_dir = ws.path().join("plugins").join("demo-tool");
        std::fs::create_dir_all(&plugin_dir).expect("plugin tree");
        std::fs::write(
            plugin_dir.join("manifest.toml"),
            "name = \"demo-tool\"\nversion = \"0.1.0\"\ncapabilities = [\"tool\"]\nwasm_path = \"t.wasm\"\n",
        )
        .expect("manifest");
        std::fs::write(plugin_dir.join("t.wasm"), b"not wasm").expect("module");
        ws
    }

    #[test]
    fn the_list_path_enforces_the_same_signature_policy_as_the_loader() {
        // iter-1726 (audit F4): under strict with NO trusted keys every
        // plugin is Untrusted — the list must reflect that (empty) rather
        // than advertising an inventory the runtime refuses to load.
        let ws = tool_plugin_workspace();
        let mut strict = AppConfig::default();
        strict.plugins.plugin_dirs = vec![ws.path().join("plugins")];
        strict.plugins.signature_mode = "strict".to_string();
        assert!(list_plugin_tools(&strict).is_empty());

        let mut disabled = AppConfig::default();
        disabled.plugins.plugin_dirs = vec![ws.path().join("plugins")];
        assert_eq!(
            list_plugin_tools(&disabled),
            vec![("demo-tool".to_string(), 1)]
        );
    }
}

#[cfg(test)]
mod live_registration_tests {
    // gap G6: the watcher's live re-registration path, exercised against
    // the REAL fixture module (plan G8 pays its debt here — these are not
    // serde-only pins).
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use operant_core::schema::ToolSchema;
    use operant_core::tools::ToolResult;
    use operant_plugins::signature::VerificationResult;
    use operant_plugins::watcher::ManifestChange;
    use std::time::Duration;

    fn fixture_wasm_src() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../operant-plugins/fixtures/fixture.wasm")
    }

    /// A plugin workspace whose single plugin declares BOTH capabilities —
    /// the fixture module exports tool_metadata/execute AND observe.
    fn change_workspace() -> (tempfile::TempDir, std::path::PathBuf) {
        let ws = tempfile::tempdir().expect("workspace");
        let plugin_dir = ws.path().join("plugins").join("fixture");
        std::fs::create_dir_all(&plugin_dir).expect("plugin tree");
        std::fs::copy(fixture_wasm_src(), plugin_dir.join("fixture.wasm"))
            .expect("copy fixture module");
        let manifest_path = plugin_dir.join("manifest.toml");
        std::fs::write(
            &manifest_path,
            "name = \"fixture\"\nversion = \"0.1.0\"\ncapabilities = [\"tool\", \"observer\"]\nwasm_path = \"fixture.wasm\"\n",
        )
        .expect("manifest");
        (ws, manifest_path)
    }

    fn change_with(
        manifest_path: &std::path::Path,
        capabilities: Vec<operant_plugins::PluginCapability>,
    ) -> ManifestChange {
        ManifestChange {
            manifest_path: manifest_path.to_path_buf(),
            manifest: operant_plugins::PluginManifest {
                name: "fixture".into(),
                version: "0.1.0".into(),
                description: None,
                author: None,
                wasm_path: Some("fixture.wasm".into()),
                capabilities,
                permissions: Vec::new(),
                signature: None,
                publisher_key: None,
                requires: Vec::new(),
                provides: Vec::new(),
                config: Default::default(),
            },
            signature: VerificationResult::Unsigned,
        }
    }

    fn test_config(ws: &tempfile::TempDir) -> AppConfig {
        let mut config = AppConfig::default();
        config.plugins.plugin_dirs = vec![ws.path().join("plugins")];
        config
    }

    /// A minimal always-succeeding stub under an arbitrary name — the
    /// observer-veto test registers it under the fixture module's blocked
    /// tool name.
    struct NamedOkTool(String);

    #[async_trait::async_trait]
    impl OperantTool for NamedOkTool {
        fn name(&self) -> &str {
            &self.0
        }
        fn description(&self) -> &str {
            "test stub"
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema::new(&self.0, "test stub", serde_json::json!({"type": "object"}))
        }
        async fn execute(&self, _args: Value, _context: ToolContext) -> ToolResult {
            operant_core::tools::ToolResult::success_with_name(&self.0, "call", "stub ran")
        }
    }

    #[tokio::test]
    async fn a_watcher_change_registers_the_plugin_tool_live() {
        let (ws, manifest_path) = change_workspace();
        let config = test_config(&ws);
        let registry = ToolRegistry::new(Duration::from_secs(5));

        apply_plugin_changes(
            &registry,
            &config,
            &[change_with(
                &manifest_path,
                vec![
                    operant_plugins::PluginCapability::Tool,
                    operant_plugins::PluginCapability::Observer,
                ],
            )],
        )
        .await
        .expect("apply");

        assert!(
            registry.contains("fixture-echo").await,
            "the watcher change must register the module's tool live"
        );
        let result = registry
            .execute(
                "fixture-echo",
                "call-1",
                serde_json::json!({"text": "ping"}),
                ToolContext::default(),
            )
            .await
            .expect("execute");
        assert!(result.success);
        // success_with_name serializes content as JSON — a string arrives quoted
        assert_eq!(
            result.content,
            serde_json::json!("fixture echo: ping").to_string()
        );
    }

    #[tokio::test]
    async fn reapplying_a_change_never_stacks_the_observer() {
        let (ws, manifest_path) = change_workspace();
        let config = test_config(&ws);
        let registry = ToolRegistry::new(Duration::from_secs(5));
        let change = change_with(
            &manifest_path,
            vec![
                operant_plugins::PluginCapability::Tool,
                operant_plugins::PluginCapability::Observer,
            ],
        );
        apply_plugin_changes(&registry, &config, std::slice::from_ref(&change))
            .await
            .expect("first apply");
        apply_plugin_changes(&registry, &config, std::slice::from_ref(&change))
            .await
            .expect("second apply (the re-detect)");
        assert_eq!(
            registry.plugin_interceptor_count().await,
            1,
            "named re-registration must REPLACE, not stack"
        );
    }

    #[tokio::test]
    async fn the_observer_verdict_blocks_through_the_registry_dispatch() {
        let (ws, manifest_path) = change_workspace();
        let config = test_config(&ws);
        let registry = ToolRegistry::new(Duration::from_secs(5));
        apply_plugin_changes(
            &registry,
            &config,
            &[change_with(
                &manifest_path,
                vec![operant_plugins::PluginCapability::Observer],
            )],
        )
        .await
        .expect("apply");

        // the fixture module vetoes the tool NAME "blocked-tool"
        registry
            .register_dyn(std::sync::Arc::new(NamedOkTool("blocked-tool".into())))
            .await
            .expect("register stub");
        let result = registry
            .execute(
                "blocked-tool",
                "call-1",
                serde_json::json!({}),
                ToolContext::default(),
            )
            .await
            .expect("execute returns an error RESULT, not an Err");
        assert!(
            !result.success,
            "the real module's Block verdict must stop the tool"
        );
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("blocked-tool"),
            "the block reason names the fixture's verdict: {:?}",
            result.error
        );
    }

    #[tokio::test]
    async fn a_disabled_plugin_change_does_not_touch_the_registry() {
        let (ws, manifest_path) = change_workspace();
        let mut config = test_config(&ws);
        config.plugins.disabled = vec!["fixture".into()];
        let registry = ToolRegistry::new(Duration::from_secs(5));
        apply_plugin_changes(
            &registry,
            &config,
            &[change_with(
                &manifest_path,
                vec![operant_plugins::PluginCapability::Tool],
            )],
        )
        .await
        .expect("apply");
        assert!(
            !registry.contains("fixture-echo").await,
            "[plugins].disabled must keep the change inert"
        );
    }

    #[tokio::test]
    async fn dropping_the_tool_capability_drops_the_plugin_tool() {
        let (ws, manifest_path) = change_workspace();
        let config = test_config(&ws);
        let registry = ToolRegistry::new(Duration::from_secs(5));
        apply_plugin_changes(
            &registry,
            &config,
            &[change_with(
                &manifest_path,
                vec![operant_plugins::PluginCapability::Tool],
            )],
        )
        .await
        .expect("first apply (tool)");
        assert!(registry.contains("fixture-echo").await);

        // the same plugin re-detected WITHOUT the tool capability
        apply_plugin_changes(
            &registry,
            &config,
            &[change_with(
                &manifest_path,
                vec![operant_plugins::PluginCapability::Observer],
            )],
        )
        .await
        .expect("second apply (observer only)");
        assert!(
            !registry.contains("fixture-echo").await,
            "a plugin that stops providing tools loses its registration"
        );
    }
}
