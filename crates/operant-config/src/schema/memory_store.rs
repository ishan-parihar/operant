//! `memory_store` configuration surface — extracted verbatim from the
//! former schema.rs monolith (dedup pass). Placement is navigational;
//! every item is re-exported from `schema::`.

use serde::{Deserialize, Serialize};

/// Route an embedding hint to a specific provider + model.
///
/// ```toml
/// [providers]
/// fallback = "primary"
///
/// [providers.models.primary]
/// base_url = "https://api.openai.com/v1"
///
/// [[providers.embedding_routes]]
/// hint = "semantic"
/// provider = "openai"
/// model = "text-embedding-3-small"
/// dimensions = 1536
/// ```
///
/// The routes live UNDER `[providers]` — a top-level `[[embedding_routes]]`
/// is rejected by the config's `deny_unknown_fields`. The
/// `[memory] embedding_model` key this example used to show is rejected
/// too: the core's `MemorySettings` has no such field, so a config
/// carrying it fails to load with "unknown field `embedding_model`".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct EmbeddingRouteConfig {
    /// Route hint name (e.g. "semantic", "archive", "faq")
    pub hint: String,
    /// Embedding provider (`none`, `openai`, or `custom:<url>`)
    pub provider: String,
    /// Embedding model to use with that provider
    pub model: String,
    /// Optional embedding dimension override for this route
    #[serde(default)]
    pub dimensions: Option<usize>,
    /// Optional API key override for this route's provider
    #[serde(default)]
    pub api_key: Option<String>,
}
