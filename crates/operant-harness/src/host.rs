//! Runtime-side host (G1) — bundles a `Harness` with the six native seams
//! and exposes ergonomic lookup helpers the agent loop can call without
//! reaching into Kernel internals.
//!
//! Constructed once at boot when `[harness].enabled = true`. The CLI
//! (`crates/operant-cli/src/main.rs`) owns the `HarnessHost` and passes it
//! to `OperantAgent::with_harness`. When `enabled = false` the agent loop
//! never sees a `HarnessHost` and the existing `ToolRegistry` /
//! `HookRunner` / `SystemPromptBuilder` paths run unchanged.

use std::sync::Arc;

use crate::claim::Claim;
use crate::composition::{Architecture, Builder};
use crate::harness::Harness;
use crate::provider::{Provider, Seam};
use crate::report::DumpTree;

/// Errors that can surface while a `HarnessHost` is being constructed or
/// while an agent is using it. Adapters translate these into the agent's
/// own `anyhow::Error` chain.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("kernel error: {0}")]
    Kernel(#[from] crate::HarnessError),
    #[error("composition error: {0}")]
    Composition(#[from] crate::composition::BuildError),
}

/// Runtime host: own the `Harness`, the active seams, and a handle to the
/// most-recent dump tree.
pub struct HarnessHost {
    harness: Arc<Harness>,
}

impl HarnessHost {
    /// Build a host from kernel options. Seams are added by the caller via
    /// [`Self::add_seam`] before any provider mounts.
    pub fn new(options: crate::KernelOptions) -> Self {
        Self {
            harness: Arc::new(Harness::new(options)),
        }
        // ponytail: Arc<Harness> + interior mutability keeps the host hand-off
        // uniform with downstream `Arc<HarnessHost>` sharing.
    }

    /// Build a host pre-wired with a `Harness` from an external Arc (the
    /// operant-pk/operant-r16 dual-kernel migration uses this to share a
    /// single kernel across multiple holders).
    pub fn with_harness(harness: Arc<Harness>) -> Self {
        Self { harness }
    }

    /// Read-only access to the underlying `Harness`.
    pub fn harness(&self) -> &Arc<Harness> {
        &self.harness
    }

    /// Register a seam sink. Must happen before providers that route
    /// installs through it are mounted.
    pub fn add_seam(&mut self, seam: Arc<dyn Seam>) {
        Arc::get_mut(&mut self.harness)
            .expect("HarnessHost::add_seam requires exclusive ownership of the Arc<Harness>")
            .add_seam(seam);
    }

    /// Boot-load an `architecture.toml` and mount every provider the
    /// `Builder` produces. Returns the set of providers that ended up
    /// active (excluding `Pending`).
    pub async fn boot(&self, arch: &Architecture) -> Result<Vec<String>, HostError> {
        let providers = Builder::build(arch)?;
        self.mount_all(providers.into_iter().map(|p| (p, serde_json::Value::Null)))
            .await
    }

    /// G6 — boot a `BuilderWithFactories` result with each row's
    /// config threaded to `mount_with_config` so seams can read the
    /// per-row config during install.
    pub async fn boot_with_factories(
        &self,
        builder: &crate::BuilderWithFactories,
        arch: &Architecture,
    ) -> Result<Vec<String>, HostError> {
        let mut pairs: Vec<(std::sync::Arc<dyn Provider>, serde_json::Value)> = Vec::new();
        for row in arch.active() {
            // Build via factory if the source needs one; else fall
            // back to the built-in Builder for native/config_row rows.
            let provider = match builder.build_with(&Architecture {
                rows: vec![row.clone()],
            }) {
                Ok(mut ps) if !ps.is_empty() => ps.pop().unwrap(),
                Ok(_) => continue, // row produced no provider
                Err(e) => return Err(e.into()),
            };
            pairs.push((provider, row.config.clone()));
        }
        self.mount_all(pairs.into_iter()).await
    }

    async fn mount_all(
        &self,
        providers: impl Iterator<Item = (std::sync::Arc<dyn Provider>, serde_json::Value)>,
    ) -> Result<Vec<String>, HostError> {
        let mut activated = Vec::new();
        for (provider, config) in providers {
            let id = provider.spec().id().to_string();
            match self.harness.mount_with_config(provider, config).await {
                Ok(crate::MountReport::Mounted { activated: mut a }) => {
                    activated.append(&mut a);
                }
                Ok(crate::MountReport::Pending { missing }) => {
                    tracing::debug!(provider = %id, ?missing, "harness provider pending (late-binding)");
                }
                Err(err) => {
                    tracing::warn!(provider = %id, error = %err, "harness provider mount failed");
                    return Err(err.into());
                }
            }
        }
        Ok(activated)
    }

    /// Hot-swap the implementation behind `provider.spec().id()`. Returns
    /// the new generation counter (or 0 if the id was unknown).
    pub async fn swap(&self, next: Arc<dyn Provider>) -> Result<u64, HostError> {
        let id = next.spec().id().to_string();
        let _ = self.harness.replace(next).await?;
        Ok(self.harness.generation_of(&id).await.unwrap_or(0))
    }

    /// Snapshot the resolved provider tree.
    pub async fn dump(&self) -> DumpTree {
        self.harness.dump().await
    }

    /// Resolve a claim owner (e.g. "who currently provides the `tool.echo`
    /// claim?") — useful for debug overlays.
    pub async fn claim_owner(&self, claim: &Claim) -> Option<String> {
        self.harness.claim_owner(claim).await
    }
}
