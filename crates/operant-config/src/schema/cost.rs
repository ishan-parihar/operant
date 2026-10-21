//! `cost` configuration surface — extracted verbatim from the
//! former schema.rs monolith (dedup pass). Placement is navigational;
//! every item is re-exported from `schema::`.

use serde::{Deserialize, Serialize};

/// Per-model pricing entry (USD per 1M tokens).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct ModelPricing {
    /// Input price per 1M tokens
    #[serde(default)]
    pub input: f64,

    /// Output price per 1M tokens
    #[serde(default)]
    pub output: f64,
}
