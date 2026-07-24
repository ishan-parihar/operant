// adapter_types/items.rs — Re-exports from decomposed sub-modules.
//
// This file previously contained 1,708 lines of mixed concerns.
// It has been decomposed into:
//   helpers.rs          — UI personalization helpers
//   auth.rs             — API credential management
//   free_catalog.rs     — Free AI provider catalog
//   model_registry.rs   — Model discovery and caching
//   anthropic_client.rs — Anthropic API client
//   provider_id.rs      — Provider identifier enum
//   tui_app.rs          — Application bootstrap and run loop

pub use super::helpers::{sample_completion_verb, sample_spinner_verb, context_window_for_model};
pub use super::auth::{AuthStore, StoredCredential};
pub use super::free_catalog::{FreeUpstream, FREE_CATALOG};
pub use super::model_registry::{ModelRegistry, RegistryModelEntry, ModelInfo};
pub use super::provider_id::ProviderId;
pub use super::tui_app::{TuiApp, LaunchMode};
