// model_picker/effort.rs — Effort levels and capability checks.
//
// Extracted from the model_picker.rs monolith. EffortLevel enum, its label
// helpers, and model_supports_effort / model_supports_max_effort checks.

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum EffortLevel {
    Low,
    #[default]
    Normal,
    High,
    Max,
}

impl EffortLevel {
    /// Unicode quarter-circle symbol used in the TS UI.
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Low => "\u{25cb}",    // ○  empty circle
            Self::Normal => "\u{25d0}", // ◐  half
            Self::High => "\u{25d5}",   // ◕  three-quarter
            Self::Max => "\u{25cf}",    // ●  full
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
            Self::Max => "max",
        }
    }
}

// ---------------------------------------------------------------------------
// Model capability helpers
// ---------------------------------------------------------------------------
