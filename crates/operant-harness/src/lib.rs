//! Harness kernel (plan 016) — Cordis-paradigm semantics on operant's Rust substrate.
//!
//! Five adopted semantics:
//! 1. String-keyed claims per typed seam (`tool`, `hook.*`, `memory.provider`, …).
//! 2. Declared requirements + PENDING late binding — providers activate the moment
//!    their dependencies appear.
//! 3. Reversible effects — every registration returns an undo handle; unload unwinds LIFO.
//! 4. Transactional swap — replace-on-success, restore-on-failure, ABA-safe generation counters.
//! 5. Composition as data — serializable dump of the resolved provider tree.
//!
//! This crate is PURE: it depends on no other operant crate. Host integration lives in
//! operant-core/operant-runtime adapters (Phase 2+).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod claim;
pub mod composition;
pub mod discovery;
pub mod effect;
pub mod error;
pub mod harness;
pub mod host;
pub mod metrics;
pub mod pool;
pub mod pool_provider;
pub mod provider;
pub mod report;
pub mod row;
pub mod swap;
pub mod wasm_provider;

pub use claim::Claim;
pub use composition::{
    Architecture, ArchitectureRow, BuildError, Builder, BuilderWithFactories, Composition, Patch,
    PatchTarget, ProviderFactory,
};
pub use discovery::{DEFAULT_PATCH_DIR, collect_patches, default_patch_dir, resolve_boot_architecture};
pub use effect::Effect;
pub use error::HarnessError;
pub use harness::{Harness, KernelOptions};
pub use host::{HarnessHost, HostError};
pub use metrics::{HarnessMetrics, MetricsSnapshot};
pub use pool::{
    CompiledPool, PoolManifest, READ_ONLY_VERBS, compile as compile_pool,
    load_and_compile as load_and_compile_pool,
};
pub use pool_provider::{PoolBundleProvider, PoolFamilyProvider};
pub use wasm_provider::WasmProvider;
pub use provider::{
    ActivateCx, Provider, ProviderSource, ProviderSpec, ProviderState, Registration, Seam, Source,
};
pub use report::{ClaimInfo, DumpTree, MountReport, ProviderEntryInfo};
pub use swap::{SwapGeneration, SwapOutcome};

/// Kernel version of the adopted-semantics contract. Bumped when a semantic changes.
pub const HARNESS_SEMANTICS_VERSION: u32 = 1;
