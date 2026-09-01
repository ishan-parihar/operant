//! Provider model — the unit of composition.
//!
//! A provider declares what it *provides* (claims) and what it *requires*
//! (dependencies). The kernel resolves load order from requirements, never
//! manual boot sequencing (Cordis `inject` semantics); a provider whose
//! requirements are unmet stays PENDING and activates the moment its
//! dependencies mount.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::claim::Claim;
use crate::effect::{BoxUndoFuture, Effect};
use crate::error::HarnessError;

/// Where a provider came from — drives trust policy and reload behavior.
///
/// G11 — `Wasm` and `Pool` carry an optional payload so the source
/// round-trips through the composition layer. The pk `Source` alias
/// remains a unit-only enum for backwards compat; new code can use
/// `ProviderSource::Wasm { path }` / `Pool { name }` to preserve
/// the per-row metadata across dumps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSource {
    /// Compiled into the binary; registered at boot.
    Native,
    /// Extism WASM module; hot-swappable. The `path` is the on-disk
    /// path to the `.wasm` file (or the manifest path).
    Wasm { path: Option<String> },
    /// Pure configuration row (no code) — e.g. a file-backed prompt section.
    ConfigRow,
    /// Compiled from an external governed manifest (hermes `_pool.yaml`).
    /// The `name` is the pool's display name.
    Pool { name: Option<String> },
}

impl ProviderSource {
    /// String-form for serialization to a row's `source` field.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderSource::Native => "native",
            ProviderSource::Wasm { .. } => "wasm",
            ProviderSource::ConfigRow => "config_row",
            ProviderSource::Pool { .. } => "pool",
        }
    }
}

impl std::fmt::Display for ProviderSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderSource::Native => f.write_str("native"),
            ProviderSource::Wasm { path } => match path {
                Some(p) => write!(f, "wasm({p})"),
                None => f.write_str("wasm"),
            },
            ProviderSource::ConfigRow => f.write_str("config-row"),
            ProviderSource::Pool { name } => match name {
                Some(n) => write!(f, "pool({n})"),
                None => f.write_str("pool"),
            },
        }
    }
}

// Manual PartialEq + Eq impls because the struct variants are not
// Copy, and serde's auto-derive is what we want for the wire form.
// (We get PartialEq + Eq from #[derive] above; this block just
// re-states that the unit-from-struct conversion preserves equality.)
impl ProviderSource {
    /// Construct a unit-style Wasm source (no path) for callers that
    /// don't have the path at hand.
    pub fn wasm() -> Self {
        ProviderSource::Wasm { path: None }
    }
    /// Construct a unit-style Pool source (no name) for callers that
    /// don't have the name at hand.
    pub fn pool() -> Self {
        ProviderSource::Pool { name: None }
    }
}

/// Backwards-compat alias for code (e.g. operant-pk) that imported the
/// `Source` type. Maps to the unit-style `ProviderSource`. G11
/// recommends new code use the explicit `ProviderSource` variants so
/// the source payload round-trips through the composition layer.
pub type Source = ProviderSource;

/// Provider lifecycle states (Cordis FiberState analog).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderState {
    /// Requirements unmet; waiting for providers to appear.
    Pending,
    /// Currently running `activate`.
    Activating,
    /// Live; claims owned; effects installed.
    Active,
    /// Unwinding.
    Unloading,
    /// Fully torn down (terminal; entry kept only in dump history).
    Disposed,
    /// Activation failed; partial effects were unwound. Kept for observability.
    Failed,
}

/// Object-safe declarative surface of a provider.
pub trait ProviderSpec: Send + Sync {
    /// Stable unique identity within this kernel instance.
    fn id(&self) -> &str;
    fn source(&self) -> ProviderSource;
    /// Claims owned on successful activation. Must be non-empty for providers
    /// that own capability addresses; empty is allowed for pure side-effect
    /// providers but they cannot be depended upon.
    fn provides(&self) -> &[Claim];
    /// Claims that must be ACTIVE before activation can run.
    fn requires(&self) -> &[Claim];
}

/// One install request routed to a seam.
#[derive(Debug)]
pub struct Registration<'a> {
    pub provider_id: &'a str,
    pub seam: &'a str,
    pub key: &'a str,
    /// Opaque per-provider configuration from the composition layer.
    pub config: &'a Value,
    /// Typed implementation object handed from provider to adapter — e.g.
    /// `Arc<dyn OperantTool>` or `Arc<dyn HookHandler>` boxed as
    /// `&(dyn Any + Send + Sync)`. Seams that install pure data (config rows,
    /// pool manifests) leave this `None` and read `config` instead.
    pub payload: Option<&'a (dyn std::any::Any + Send + Sync)>,
}

/// A seam sink: the host-side half of a capability family. The kernel routes
/// installs through seams; adapters (Phase 2+) translate them into real
/// registrations against ToolRegistry / HookRunner / gateway registries.
///
/// Implementations MUST NOT call back into [`crate::Harness`] lifecycle
/// methods from `install` — lifecycle ops hold the kernel write lock.
#[async_trait::async_trait]
pub trait Seam: Send + Sync {
    /// Seam family name (matches `Claim.seam`).
    fn name(&self) -> &str;

    /// Install one registration and return its undo effect.
    async fn install(&self, reg: &Registration<'_>) -> Result<Effect, HarnessError>;
}

/// G6 — typed payload that a provider installs through the `tool` seam.
/// The host's `ToolSeam` (or any tool seam) calls `tool_name()` to
/// register the underlying `Arc<dyn OperantTool>` under that name in
/// the `ToolRegistry`. The payload's `path`/`read_only` fields are
/// used by the `PoolBundleProvider` adapter.
pub trait SeamToolPayload: Send + Sync {
    fn tool_name(&self) -> &str;
}

/// Handed to [`Provider::activate`]. Collects effects; every successful
/// `install` pushes an undo handle that the kernel owns from then on.
pub struct ActivateCx<'a> {
    provider_id: &'a str,
    config: &'a Value,
    seams: &'a std::collections::HashMap<String, std::sync::Arc<dyn Seam>>,
    effects: &'a mut Vec<Effect>,
}

impl<'a> ActivateCx<'a> {
    pub(crate) fn new(
        provider_id: &'a str,
        config: &'a Value,
        seams: &'a std::collections::HashMap<String, std::sync::Arc<dyn Seam>>,
        effects: &'a mut Vec<Effect>,
    ) -> Self {
        Self {
            provider_id,
            config,
            seams,
            effects,
        }
    }

    pub fn provider_id(&self) -> &str {
        self.provider_id
    }

    /// Per-provider configuration (composition row `config`, or Null).
    pub fn config(&self) -> &Value {
        self.config
    }

    /// Push a manually-constructed effect (for registrations made outside
    /// seam routing).
    pub fn push_effect(
        &mut self,
        label: impl Into<String>,
        undo: impl FnOnce() -> BoxUndoFuture + Send + Sync + 'static,
    ) {
        self.effects.push(Effect::new(label, undo));
    }

    /// Route one install through the named seam. On success the returned
    /// undo effect is retained by the kernel; on failure previously collected
    /// effects are unwound by the kernel (partial-failure containment).
    pub async fn install(&mut self, seam: &str, key: &str) -> Result<(), HarnessError> {
        let sink = self
            .seams
            .get(seam)
            .ok_or_else(|| HarnessError::MissingSeam(seam.to_string()))?;
        let reg = Registration {
            provider_id: self.provider_id,
            seam,
            key,
            config: self.config,
            payload: None,
        };
        let effect = sink.install(&reg).await?;
        tracing::debug!(provider = %self.provider_id, seam, key, "harness install");
        self.effects.push(effect);
        Ok(())
    }

    /// Route one install through the named seam, carrying a typed payload
    /// object (see [`Registration::payload`]). Same effect bookkeeping as
    /// [`Self::install`].
    pub async fn install_with(
        &mut self,
        seam: &str,
        key: &str,
        payload: &(dyn std::any::Any + Send + Sync),
    ) -> Result<(), HarnessError> {
        let sink = self
            .seams
            .get(seam)
            .ok_or_else(|| HarnessError::MissingSeam(seam.to_string()))?;
        let reg = Registration {
            provider_id: self.provider_id,
            seam,
            key,
            config: self.config,
            payload: Some(payload),
        };
        let effect = sink.install(&reg).await?;
        tracing::debug!(provider = %self.provider_id, seam, key, "harness install (payload)");
        self.effects.push(effect);
        Ok(())
    }
}

/// A composable extension unit. Implementations are stored as
/// `Arc<dyn Provider>`; keep methods object-safe.
#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn spec(&self) -> &dyn ProviderSpec;

    /// Bring the provider live by installing through the context. On `Err`,
    /// the kernel unwinds any partially collected effects and records the
    /// entry as Failed — the rest of the harness is untouched.
    async fn activate(&self, cx: &mut ActivateCx<'_>) -> Result<(), HarnessError>;
}
