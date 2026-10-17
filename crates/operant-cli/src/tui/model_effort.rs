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

    /// Cycle to next level; skips `Max` when the selected model does not
    /// support it.
    pub fn next(self, supports_max: bool) -> Self {
        match self {
            Self::Low => Self::Normal,
            Self::Normal => Self::High,
            Self::High => {
                if supports_max {
                    Self::Max
                } else {
                    Self::Low
                }
            }
            Self::Max => Self::Low,
        }
    }

    /// Cycle to previous level.
    pub fn prev(self, supports_max: bool) -> Self {
        match self {
            Self::Low => {
                if supports_max {
                    Self::Max
                } else {
                    Self::High
                }
            }
            Self::Normal => Self::Low,
            Self::High => Self::Normal,
            Self::Max => Self::High,
        }
    }
}

// ---------------------------------------------------------------------------
// Model capability helpers
// ---------------------------------------------------------------------------

/// Returns `true` for models that support extended thinking / effort levels.
pub fn model_supports_effort(id: &str) -> bool {
    id.starts_with("claude-3-7")
        || id.starts_with("claude-opus-4")
        || id.starts_with("claude-sonnet-4")
}

/// Returns `true` for models that support the maximum effort tier.
pub fn model_supports_max_effort(id: &str) -> bool {
    id.starts_with("claude-opus-4")
}
