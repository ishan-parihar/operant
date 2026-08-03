//! Cost tracking: token usage records, aggregation, and budget enforcement.

/// Session/period cost aggregation with budget enforcement.
pub mod tracker;
/// Token usage records, summaries, and budget-check types.
pub mod types;
pub use tracker::CostTracker;
pub use types::*;
