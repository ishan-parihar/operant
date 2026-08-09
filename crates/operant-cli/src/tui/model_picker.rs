//! Model picker overlay (/model command).
//! Mirrors src/components/ModelPicker.tsx — including effort levels and
//! fast-mode notice.

use crate::tui::overlays::{
    OPERANT_PANEL_BG, centered_rect, cycle_next, cycle_prev, modal_search_line,
};

// ---------------------------------------------------------------------------
// Effort level
// ---------------------------------------------------------------------------

/// Mirrors the TS `EffortLevel` enum and `effortLevelToSymbol()` helper.
///
/// Effort controls the extended-thinking `budget_tokens` parameter sent to the
/// API. Only models that support extended thinking honour this; for others it
/// is silently ignored.
mod effort;
mod models;
mod render;
mod state;

#[cfg(test)]
mod tests;

pub use effort::{EffortLevel, model_supports_effort, model_supports_max_effort};
pub(crate) use models::model_entry;
pub use models::{ModelEntry, default_model_for_provider, models_for_provider_from_registry};
pub use render::render_model_picker;
pub use state::ModelPickerState;
